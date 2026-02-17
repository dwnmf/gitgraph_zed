use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::actions::{ActionCatalog, ActionContext, ActionRequest, ResolvedAction};
use crate::error::{GitLgError, Result};
use crate::git::{GitOutput, GitRunner};
use crate::log_parser::{FIELD_SEP, build_graph_rows, parse_git_log_records};
use crate::models::{BlameInfo, BranchInfo, CommitSearchQuery, FileChange, GraphData, GraphQuery};
use crate::search::filter_commits;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionExecutionResult {
    pub action_id: String,
    pub command_line: String,
    pub args: Vec<String>,
    pub output: GitOutput,
}

#[derive(Debug, Clone)]
pub struct GitLgService {
    git: GitRunner,
    actions: ActionCatalog,
}

impl GitLgService {
    pub fn new(git: GitRunner, actions: ActionCatalog) -> Self {
        Self { git, actions }
    }

    pub fn with_default_actions(git: GitRunner) -> Self {
        Self::new(git, ActionCatalog::with_defaults())
    }

    pub fn actions(&self) -> &ActionCatalog {
        &self.actions
    }

    pub fn graph(&self, repo_path: &Path, query: &GraphQuery) -> Result<GraphData> {
        self.git.validate_repo(repo_path)?;
        let log_output = self.run_log(repo_path, query)?;
        let commits = build_graph_rows(parse_git_log_records(&log_output.stdout)?);
        let branches = self.read_branches(repo_path)?;
        Ok(GraphData {
            repository: normalize_repo_path(repo_path),
            generated_at_unix: current_unix_timestamp(),
            query: query.clone(),
            commits,
            branches,
        })
    }

    pub fn graph_filtered(
        &self,
        repo_path: &Path,
        query: &GraphQuery,
        search_query: &CommitSearchQuery,
    ) -> Result<GraphData> {
        let mut data = self.graph(repo_path, query)?;
        let has_search_text = !search_query.text.trim().is_empty();
        if let Some(file_path) = search_query
            .file_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            data.commits = self.filter_commits_by_file_contents(
                repo_path,
                data.commits,
                search_query,
                file_path,
            )?;
        } else if has_search_text {
            data.commits = filter_commits(&data.commits, search_query)?;
        }
        Ok(data)
    }

    pub fn execute_action(
        &self,
        repo_path: &Path,
        request: ActionRequest,
        default_remote_name: &str,
    ) -> Result<ActionExecutionResult> {
        let request = normalize_action_request(request, &self.actions, default_remote_name);
        let resolved = self.actions.resolve_with_lookup(request, |placeholder| {
            self.lookup_dynamic_placeholder(repo_path, placeholder)
        })?;
        let output = if let Some(script) = &resolved.shell_script {
            let script = format!("{} {}", self.git.git_binary(), script);
            self.git
                .exec_shell(repo_path, &script, resolved.allow_non_zero_exit)?
        } else {
            self.git
                .exec(repo_path, &resolved.args, resolved.allow_non_zero_exit)?
        };
        Ok(ActionExecutionResult {
            action_id: resolved.id,
            command_line: resolved.command_line,
            args: resolved.args,
            output,
        })
    }

    pub fn resolve_action_preview(
        &self,
        request: ActionRequest,
        default_remote_name: &str,
        repo_path: Option<&Path>,
    ) -> Result<ResolvedAction> {
        let request = normalize_action_request(request, &self.actions, default_remote_name);
        self.actions.resolve_with_lookup(request, |placeholder| {
            if let Some(repo_path) = repo_path {
                return self.lookup_dynamic_placeholder(repo_path, placeholder);
            }
            Ok(None)
        })
    }

    pub fn blame_line(&self, repo_path: &Path, file: &Path, line: usize) -> Result<BlameInfo> {
        let line_no = line.max(1);
        let repo = normalize_repo_path(repo_path);
        let repo_file = normalize_repo_file_input(&repo, file);
        let out = self.git.exec(
            &repo,
            &[
                "blame".to_string(),
                format!("-L{line_no},{line_no}"),
                "--porcelain".to_string(),
                "--".to_string(),
                repo_file.to_string_lossy().to_string(),
            ],
            false,
        )?;
        let mut commit_hash = String::new();
        let mut author_name = String::new();
        let mut author_email = String::new();
        let mut author_time_unix: i64 = 0;
        let mut summary = String::new();
        for (idx, line) in out.stdout.lines().enumerate() {
            if idx == 0 {
                commit_hash = line
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                continue;
            }
            if let Some(v) = line.strip_prefix("author ") {
                author_name = v.to_string();
                continue;
            }
            if let Some(v) = line.strip_prefix("author-mail ") {
                author_email = v.trim_matches(['<', '>']).to_string();
                continue;
            }
            if let Some(v) = line.strip_prefix("author-time ") {
                author_time_unix = v.parse::<i64>().unwrap_or(0);
                continue;
            }
            if let Some(v) = line.strip_prefix("summary ") {
                summary = v.to_string();
                continue;
            }
        }

        Ok(BlameInfo {
            file: repo.join(repo_file),
            line: line_no,
            commit_hash,
            author_name,
            author_email,
            author_time_unix,
            summary,
        })
    }

    pub fn commit_file_changes(
        &self,
        repo_path: &Path,
        commit_hash: &str,
    ) -> Result<Vec<FileChange>> {
        self.git.validate_repo(repo_path)?;
        let out = self.git.exec(
            repo_path,
            &[
                "-c".to_string(),
                "color.ui=never".to_string(),
                "-c".to_string(),
                "core.quotePath=false".to_string(),
                "show".to_string(),
                "--numstat".to_string(),
                "--no-color".to_string(),
                "--no-ext-diff".to_string(),
                "--format=".to_string(),
                "--find-renames".to_string(),
                "--find-copies".to_string(),
                commit_hash.to_string(),
            ],
            false,
        )?;
        let mut files = Vec::new();
        for line in out.stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut parts = trimmed.splitn(3, '\t');
            let Some(added_raw) = parts.next() else {
                continue;
            };
            let Some(removed_raw) = parts.next() else {
                continue;
            };
            let Some(path) = parts.next() else {
                continue;
            };

            files.push(FileChange {
                path: path.to_string(),
                added: parse_numstat_value(added_raw),
                removed: parse_numstat_value(removed_raw),
            });
        }
        Ok(files)
    }

    pub fn commit_file_patch(
        &self,
        repo_path: &Path,
        commit_hash: &str,
        file_path: &str,
        context_lines: usize,
    ) -> Result<String> {
        self.git.validate_repo(repo_path)?;
        let normalized_path = normalize_numstat_path(file_path);
        let mut candidate_paths = vec![file_path.trim().to_string()];
        if normalized_path != file_path.trim() {
            candidate_paths.push(normalized_path);
        }

        for candidate in candidate_paths {
            if candidate.is_empty() {
                continue;
            }
            if let Some(patch) = self.try_commit_file_patch(
                repo_path,
                commit_hash,
                &candidate,
                context_lines,
                false,
            )? {
                return Ok(patch);
            }
            if let Some(patch) =
                self.try_commit_file_patch(repo_path, commit_hash, &candidate, context_lines, true)?
            {
                return Ok(patch);
            }
        }

        Ok(String::new())
    }

    fn try_commit_file_patch(
        &self,
        repo_path: &Path,
        commit_hash: &str,
        file_path: &str,
        context_lines: usize,
        split_merge_parents: bool,
    ) -> Result<Option<String>> {
        let mut args = vec![
            "-c".to_string(),
            "color.ui=never".to_string(),
            "-c".to_string(),
            "core.quotePath=false".to_string(),
            "show".to_string(),
            "--patch".to_string(),
            "--no-color".to_string(),
            "--no-ext-diff".to_string(),
            "--format=".to_string(),
            "--find-renames".to_string(),
            "--find-copies".to_string(),
            format!("--unified={context_lines}"),
        ];
        if split_merge_parents {
            args.push("-m".to_string());
        }
        args.push(commit_hash.to_string());
        args.push("--".to_string());
        args.push(file_path.to_string());

        let out = self.git.exec(repo_path, &args, false)?;
        if out.stdout.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(out.stdout))
    }

    fn run_log(&self, repo_path: &Path, query: &GraphQuery) -> Result<GitOutput> {
        let mut args = vec![
            "-c".to_string(),
            "color.ui=never".to_string(),
            "log".to_string(),
            "--date-order".to_string(),
            "--topo-order".to_string(),
            "--decorate=full".to_string(),
            "--color=never".to_string(),
            format!(
                "--pretty=format:%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%ct%x1f%D%x1f%s%x1f%b%x1e"
            ),
            "--no-show-signature".to_string(),
            "--no-notes".to_string(),
            "-n".to_string(),
            query.limit.to_string(),
            "--skip".to_string(),
            query.skip.to_string(),
        ];

        if query.all_refs {
            args.push("--all".to_string());
        }
        if query.include_stash_ref && self.has_stash_ref(repo_path)? {
            args.push("refs/stash".to_string());
        }
        args.extend(query.additional_args.clone());
        self.git.exec(repo_path, &args, false)
    }

    fn has_stash_ref(&self, repo_path: &Path) -> Result<bool> {
        let out = self.git.exec(
            repo_path,
            &[
                "show-ref".to_string(),
                "--verify".to_string(),
                "--quiet".to_string(),
                "refs/stash".to_string(),
            ],
            true,
        )?;
        Ok(out.exit_code == Some(0))
    }

    fn read_branches(&self, repo_path: &Path) -> Result<Vec<BranchInfo>> {
        let out = self.git.exec(
            repo_path,
            &[
                "branch".to_string(),
                "--list".to_string(),
                "--all".to_string(),
                "--sort=-committerdate".to_string(),
                format!("--format=%(upstream:remotename){FIELD_SEP}%(refname)"),
            ],
            false,
        )?;
        let branches = out
            .stdout
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                let mut parts = trimmed.splitn(2, FIELD_SEP);
                let remote_name = parts.next().map(str::trim).unwrap_or_default();
                let full_ref = parts.next().map(str::trim).unwrap_or_default();
                if full_ref.is_empty() {
                    return None;
                }
                let is_remote = full_ref.starts_with("refs/remotes/");
                let name = full_ref
                    .strip_prefix("refs/heads/")
                    .or_else(|| full_ref.strip_prefix("refs/remotes/"))
                    .unwrap_or(full_ref)
                    .to_string();
                Some(BranchInfo {
                    name,
                    full_ref: full_ref.to_string(),
                    is_remote,
                    remote_name: if remote_name.is_empty() {
                        None
                    } else {
                        Some(remote_name.to_string())
                    },
                })
            })
            .collect();
        Ok(branches)
    }

    fn lookup_dynamic_placeholder(
        &self,
        repo_path: &Path,
        placeholder: &str,
    ) -> Result<Option<String>> {
        if let Some(key) = placeholder.strip_prefix("GIT_CONFIG:") {
            let out = self.git.exec(
                repo_path,
                &["config".to_string(), "--get".to_string(), key.to_string()],
                true,
            )?;
            return Ok(Some(out.stdout.trim().to_string()));
        }
        if let Some(raw_args) = placeholder.strip_prefix("GIT_EXEC:") {
            let args = shlex::split(raw_args).unwrap_or_else(|| {
                raw_args
                    .split_whitespace()
                    .map(ToString::to_string)
                    .collect()
            });
            if args.is_empty() {
                return Ok(Some(String::new()));
            }
            let out = self.git.exec(repo_path, &args, true)?;
            return Ok(Some(out.stdout.trim().to_string()));
        }
        Ok(None)
    }

    fn filter_commits_by_file_contents(
        &self,
        repo_path: &Path,
        rows: Vec<crate::models::GraphRow>,
        search_query: &CommitSearchQuery,
        file_path: &str,
    ) -> Result<Vec<crate::models::GraphRow>> {
        if rows.is_empty() || search_query.text.trim().is_empty() {
            return Ok(rows);
        }

        let normalized_path = file_path.replace('\\', "/");
        let mut matched_hashes = HashSet::new();
        for chunk in rows.chunks(200) {
            let mut args = vec!["grep".to_string()];
            if search_query.use_regex {
                args.push("-E".to_string());
            } else {
                args.push("-F".to_string());
            }
            if !search_query.case_sensitive {
                args.push("-i".to_string());
            }
            args.push("-n".to_string());
            args.push("-e".to_string());
            args.push(search_query.text.clone());
            args.extend(chunk.iter().map(|row| row.hash.clone()));
            args.push("--".to_string());
            args.push(normalized_path.clone());

            let out = self.git.exec(repo_path, &args, true)?;
            if !matches!(out.exit_code, Some(0) | Some(1)) {
                return Err(GitLgError::GitCommandFailed {
                    program: self.git.git_binary().to_string(),
                    args,
                    exit_code: out.exit_code,
                    stderr: out.stderr,
                    stdout: out.stdout,
                });
            }
            for line in out.stdout.lines() {
                if let Some((hash, _)) = line.split_once(':') {
                    matched_hashes.insert(hash.to_string());
                }
            }
        }

        if matched_hashes.is_empty() {
            return Ok(Vec::new());
        }
        Ok(rows
            .into_iter()
            .filter(|row| matched_hashes.contains(&row.hash))
            .collect())
    }
}

fn normalize_repo_path(repo_path: &Path) -> PathBuf {
    repo_path
        .canonicalize()
        .unwrap_or_else(|_| repo_path.to_path_buf())
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn merge_default_context(mut context: ActionContext, default_remote_name: &str) -> ActionContext {
    if context.remote_name.is_none() {
        context.remote_name = Some(default_remote_name.to_string());
    }
    if context.default_remote_name.is_none() {
        context.default_remote_name = Some(default_remote_name.to_string());
    }
    context
}

fn normalize_action_request(
    mut request: ActionRequest,
    catalog: &ActionCatalog,
    default_remote_name: &str,
) -> ActionRequest {
    request.context = merge_default_context(request.context, default_remote_name);
    if request.template_id.contains(':') {
        return request;
    }
    if let Some(best_id) = choose_template_for_short_id(
        catalog,
        &request.template_id,
        &request.context,
        &request.params,
    ) {
        request.template_id = best_id;
    }
    request
}

fn choose_template_for_short_id(
    catalog: &ActionCatalog,
    short_id: &str,
    context: &ActionContext,
    params: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let mut candidates: Vec<_> = catalog
        .templates
        .iter()
        .filter(|t| t.id.ends_with(&format!(":{short_id}")))
        .collect();
    if candidates.is_empty() {
        candidates = catalog
            .templates
            .iter()
            .filter(|t| {
                let title = sanitize_id_fragment(&t.title);
                title == short_id || title.starts_with(&format!("{short_id}-"))
            })
            .collect();
    }
    if candidates.is_empty() {
        candidates = catalog
            .templates
            .iter()
            .filter(|t| {
                t.args
                    .first()
                    .is_some_and(|cmd| cmd.eq_ignore_ascii_case(short_id))
            })
            .collect();
    }
    if candidates.is_empty() {
        return None;
    }
    let mut available = context.to_placeholder_map();
    available.extend(params.clone());
    let placeholder_regex = Regex::new(r"\{([^}]+)\}").expect("regex compiles");

    candidates
        .into_iter()
        .min_by_key(|t| {
            let mut missing = 0usize;
            let mut check_text = t.args.join(" ");
            check_text.push(' ');
            check_text.push_str(&t.raw_args);
            for param in &t.params {
                check_text.push(' ');
                check_text.push_str(&param.default_value);
            }
            for cap in placeholder_regex.captures_iter(&check_text) {
                let Some(name_match) = cap.get(1) else {
                    continue;
                };
                let name = name_match.as_str();
                if name.starts_with("GIT_CONFIG:") || name.starts_with("GIT_EXEC:") {
                    continue;
                }
                if !available.contains_key(name) {
                    missing += 1;
                }
            }
            (missing, t.shell_script, t.params.len(), t.args.len())
        })
        .map(|t| t.id.clone())
}

fn sanitize_id_fragment(text: &str) -> String {
    let lowered = text.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut prev_dash = false;
    for ch in lowered.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn normalize_repo_file_input(repo_root: &Path, file: &Path) -> PathBuf {
    if file.is_absolute() {
        return file
            .strip_prefix(repo_root)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|_| file.to_path_buf());
    }
    file.to_path_buf()
}

fn parse_numstat_value(raw: &str) -> Option<u32> {
    if raw == "-" {
        return None;
    }
    raw.parse::<u32>().ok()
}

fn normalize_numstat_path(raw: &str) -> String {
    let mut path = raw.trim().trim_matches('"').to_string();
    if path.is_empty() {
        return path;
    }

    if path.contains('{') && path.contains(" => ") {
        let chars = path.chars().collect::<Vec<_>>();
        let mut out = String::with_capacity(path.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '{'
                && let Some(close) = chars[i + 1..].iter().position(|c| *c == '}')
            {
                let end = i + 1 + close;
                let inner = chars[i + 1..end].iter().collect::<String>();
                if let Some((_, rhs)) = inner.split_once(" => ") {
                    out.push_str(rhs.trim());
                    i = end + 1;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        path = out;
    }

    if let Some((_, rhs)) = path.rsplit_once(" => ") {
        path = rhs.trim().to_string();
    }

    path
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::process::Command;

    use tempfile::TempDir;

    use crate::actions::{ActionContext, ActionRequest};
    use crate::models::{CommitSearchQuery, GraphQuery};

    use super::GitLgService;
    use super::GitRunner;

    fn has_git() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn init_repo(tmp: &TempDir) {
        Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .expect("git init");
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(tmp.path())
            .output()
            .expect("config user.name");
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(tmp.path())
            .output()
            .expect("config user.email");
        fs::write(tmp.path().join("a.txt"), "a\n").expect("write a");
        Command::new("git")
            .args(["add", "."])
            .current_dir(tmp.path())
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(tmp.path())
            .output()
            .expect("git commit");
    }

    fn commit_file(tmp: &TempDir, path: &str, content: &str, message: &str) {
        fs::write(tmp.path().join(path), content).expect("write file");
        Command::new("git")
            .args(["add", path])
            .current_dir(tmp.path())
            .output()
            .expect("git add file");
        Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(tmp.path())
            .output()
            .expect("git commit file");
    }

    #[test]
    fn can_load_graph() {
        if !has_git() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        init_repo(&tmp);

        let service = GitLgService::with_default_actions(GitRunner::default());
        let graph = service
            .graph(tmp.path(), &GraphQuery::default())
            .expect("graph builds");
        assert!(!graph.commits.is_empty());
        assert_eq!(graph.commits[0].subject, "init");
    }

    #[test]
    fn can_execute_checkout_action_preview() {
        if !has_git() {
            return;
        }
        let service = GitLgService::with_default_actions(GitRunner::default());
        let request = ActionRequest {
            template_id: "checkout".to_string(),
            params: HashMap::new(),
            enabled_options: HashSet::new(),
            context: ActionContext {
                branch_name: Some("main".to_string()),
                ..ActionContext::default()
            },
        };
        let preview = service
            .resolve_action_preview(request, "origin", None)
            .expect("resolves");
        assert_eq!(preview.args, vec!["checkout", "main"]);
    }

    #[test]
    fn can_blame_line() {
        if !has_git() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        init_repo(&tmp);
        let service = GitLgService::with_default_actions(GitRunner::default());
        let blame = service
            .blame_line(tmp.path(), &tmp.path().join("a.txt"), 1)
            .expect("blame line");
        assert!(!blame.commit_hash.is_empty());
        assert_eq!(blame.author_name, "Test");
    }

    #[test]
    fn can_search_file_contents_in_history() {
        if !has_git() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        init_repo(&tmp);
        commit_file(&tmp, "notes.txt", "needle in a stack\n", "add notes");
        commit_file(&tmp, "notes.txt", "clean line\n", "remove needle");

        let service = GitLgService::with_default_actions(GitRunner::default());
        let graph = service
            .graph_filtered(
                tmp.path(),
                &GraphQuery::default(),
                &CommitSearchQuery {
                    text: "needle".to_string(),
                    file_path: Some("notes.txt".to_string()),
                    ..CommitSearchQuery::default()
                },
            )
            .expect("graph filtered");
        assert_eq!(graph.commits.len(), 1);
        assert_eq!(graph.commits[0].subject, "add notes");
    }

    #[test]
    fn short_id_merge_prefers_merge_template() {
        let service = GitLgService::with_default_actions(GitRunner::default());
        let preview = service
            .resolve_action_preview(
                ActionRequest {
                    template_id: "merge".to_string(),
                    params: HashMap::new(),
                    enabled_options: HashSet::new(),
                    context: ActionContext {
                        branch_display_name: Some("feature/my-work".to_string()),
                        ..ActionContext::default()
                    },
                },
                "origin",
                None,
            )
            .expect("resolve merge");
        assert_eq!(preview.args.first().map(String::as_str), Some("merge"));
        assert!(preview.command_line.contains("feature/my-work"));
    }

    #[test]
    fn can_read_commit_file_changes_and_patch() {
        if !has_git() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        init_repo(&tmp);
        commit_file(&tmp, "notes.txt", "line one\nline two\n", "add notes");

        let service = GitLgService::with_default_actions(GitRunner::default());
        let graph = service
            .graph(
                tmp.path(),
                &GraphQuery {
                    limit: 1,
                    ..GraphQuery::default()
                },
            )
            .expect("graph");
        let commit_hash = graph.commits[0].hash.clone();
        let files = service
            .commit_file_changes(tmp.path(), &commit_hash)
            .expect("file changes");
        assert!(files.iter().any(|f| f.path.ends_with("notes.txt")));

        let patch = service
            .commit_file_patch(tmp.path(), &commit_hash, "notes.txt", 3)
            .expect("patch");
        assert!(patch.contains("+line one"));
    }

    #[test]
    fn normalize_numstat_paths_for_renames() {
        assert_eq!(super::normalize_numstat_path("README.md"), "README.md");
        assert_eq!(
            super::normalize_numstat_path("old.txt => new.txt"),
            "new.txt"
        );
        assert_eq!(
            super::normalize_numstat_path("src/{old => new}/mod.rs"),
            "src/new/mod.rs"
        );
        assert_eq!(
            super::normalize_numstat_path("\"src/{old => new}/mod.rs\""),
            "src/new/mod.rs"
        );
    }
}
