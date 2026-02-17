use std::collections::{HashMap, HashSet};

use gitgraph_core::log_parser::{build_graph_rows, parse_git_log_records};
use gitgraph_core::{
    ActionCatalog, ActionContext, ActionRequest, ActionScope, CommitSearchQuery, GitLgError,
    filter_commits,
};
use zed_extension_api as zed;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1_000;

fn is_log_command(name: &str) -> bool {
    matches!(name, "gitgraph-log" | "gitlg-log")
}

fn is_search_command(name: &str) -> bool {
    matches!(name, "gitgraph-search" | "gitlg-search")
}

fn is_actions_command(name: &str) -> bool {
    matches!(name, "gitgraph-actions" | "gitlg-actions")
}

fn is_action_command(name: &str) -> bool {
    matches!(name, "gitgraph-action" | "gitlg-action")
}

fn is_blame_command(name: &str) -> bool {
    matches!(name, "gitgraph-blame" | "gitlg-blame")
}

fn is_tips_command(name: &str) -> bool {
    matches!(name, "gitgraph-tips" | "gitlg-tips")
}

struct GitGraphZedExtension;

impl zed::Extension for GitGraphZedExtension {
    fn new() -> Self {
        Self
    }

    fn complete_slash_command_argument(
        &self,
        command: zed::SlashCommand,
        _args: Vec<String>,
    ) -> zed::Result<Vec<zed::SlashCommandArgumentCompletion>, String> {
        if is_log_command(command.name.as_str()) || is_search_command(command.name.as_str()) {
            let mut items = vec![
                zed::SlashCommandArgumentCompletion {
                    label: "limit=25".to_string(),
                    new_text: "limit=25".to_string(),
                    run_command: false,
                },
                zed::SlashCommandArgumentCompletion {
                    label: "limit=100".to_string(),
                    new_text: "limit=100".to_string(),
                    run_command: false,
                },
                zed::SlashCommandArgumentCompletion {
                    label: "limit=250".to_string(),
                    new_text: "limit=250".to_string(),
                    run_command: false,
                },
            ];
            if is_search_command(command.name.as_str()) {
                items.push(zed::SlashCommandArgumentCompletion {
                    label: "path=src/main.rs".to_string(),
                    new_text: "path=src/main.rs".to_string(),
                    run_command: false,
                });
            }
            return Ok(items);
        }
        if is_action_command(command.name.as_str()) {
            let completions = ActionCatalog::with_defaults()
                .templates
                .iter()
                .map(|t| zed::SlashCommandArgumentCompletion {
                    label: format!("{} ({})", t.id, t.scope.as_str()),
                    new_text: t.id.clone(),
                    run_command: false,
                })
                .collect();
            return Ok(completions);
        }
        if is_blame_command(command.name.as_str()) {
            return Ok(vec![
                zed::SlashCommandArgumentCompletion {
                    label: "README.md 1".to_string(),
                    new_text: "README.md 1".to_string(),
                    run_command: false,
                },
                zed::SlashCommandArgumentCompletion {
                    label: "src/main.rs 42".to_string(),
                    new_text: "src/main.rs 42".to_string(),
                    run_command: false,
                },
            ]);
        }
        Ok(Vec::new())
    }

    fn run_slash_command(
        &self,
        command: zed::SlashCommand,
        args: Vec<String>,
        worktree: Option<&zed::Worktree>,
    ) -> zed::Result<zed::SlashCommandOutput, String> {
        let worktree = worktree.ok_or_else(|| {
            format!(
                "{} requires a project worktree context (open a repository first)",
                command.name
            )
        })?;
        let root = worktree.root_path();
        let command_name = command.name.as_str();
        if is_log_command(command_name) {
            return run_gitgraph_log(&root, args);
        }
        if is_search_command(command_name) {
            return run_gitgraph_search(&root, args);
        }
        if is_actions_command(command_name) {
            return run_gitgraph_actions();
        }
        if is_action_command(command_name) {
            return run_gitgraph_action(&root, args);
        }
        if is_blame_command(command_name) {
            return run_gitgraph_blame(&root, args);
        }
        if is_tips_command(command_name) {
            return run_gitgraph_tips();
        }
        Err(format!("unsupported slash command: {}", command_name))
    }
}

fn run_gitgraph_log(repo_root: &str, args: Vec<String>) -> Result<zed::SlashCommandOutput, String> {
    let limit = parse_limit_arg(args.first().map(String::as_str))?;
    let output = run_git_log(repo_root, limit)?;
    let rows = build_graph_rows(
        parse_git_log_records(&output).map_err(|e| format!("failed to parse git output: {e}"))?,
    );
    let text = render_rows(
        repo_root,
        &rows,
        &format!("Showing {} commit(s)", rows.len()),
    );
    Ok(build_output(text, "GitGraph graph"))
}

fn run_gitgraph_search(
    repo_root: &str,
    args: Vec<String>,
) -> Result<zed::SlashCommandOutput, String> {
    let parsed = parse_search_args(args)?;
    let output = run_git_log(repo_root, parsed.limit)?;
    let rows = build_graph_rows(
        parse_git_log_records(&output).map_err(|e| format!("failed to parse git output: {e}"))?,
    );
    let search = CommitSearchQuery {
        text: parsed.query.clone(),
        file_path: parsed.file_path.clone(),
        ..CommitSearchQuery::default()
    };
    let filtered = if let Some(path) = parsed.file_path.as_deref() {
        filter_rows_by_file_contents(repo_root, &rows, &search, path)?
    } else {
        filter_commits(&rows, &search).map_err(|e| format!("search failed: {e}"))?
    };
    let text = render_rows(
        repo_root,
        &filtered,
        &format!(
            "Matched {} commit(s) from {} scanned",
            filtered.len(),
            rows.len()
        ),
    );
    Ok(build_output(text, "GitGraph search"))
}

fn filter_rows_by_file_contents(
    repo_root: &str,
    rows: &[gitgraph_core::GraphRow],
    search: &CommitSearchQuery,
    file_path: &str,
) -> Result<Vec<gitgraph_core::GraphRow>, String> {
    if rows.is_empty() || search.text.trim().is_empty() {
        return Ok(rows.to_vec());
    }

    let normalized = file_path.replace('\\', "/");
    let mut matched_hashes = HashSet::new();
    for chunk in rows.chunks(200) {
        let mut args = vec!["grep".to_string()];
        if search.use_regex {
            args.push("-E".to_string());
        } else {
            args.push("-F".to_string());
        }
        if !search.case_sensitive {
            args.push("-i".to_string());
        }
        args.push("-n".to_string());
        args.push("-e".to_string());
        args.push(search.text.clone());
        args.extend(chunk.iter().map(|row| row.hash.clone()));
        args.push("--".to_string());
        args.push(normalized.clone());

        let out = run_git_command(repo_root, &args)?;
        if !matches!(out.status, Some(0) | Some(1)) {
            return Err(format!(
                "git grep failed (exit {:?}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some((hash, _)) = line.split_once(':') {
                matched_hashes.insert(hash.to_string());
            }
        }
    }

    Ok(rows
        .iter()
        .filter(|row| matched_hashes.contains(&row.hash))
        .cloned()
        .collect())
}

fn run_gitgraph_actions() -> Result<zed::SlashCommandOutput, String> {
    let catalog = ActionCatalog::with_defaults();
    let mut text = String::new();
    text.push_str("# GitGraph actions\n\n");
    for scope in ActionScope::all() {
        let templates = catalog.templates_for_scope(*scope);
        text.push_str(&format!("## {} ({})\n", scope.as_str(), templates.len()));
        for t in templates {
            text.push_str(&format!(
                "- `{}`: {} -> `{}`\n",
                t.id,
                t.title,
                t.args.join(" ")
            ));
        }
        text.push('\n');
    }
    Ok(build_output(text, "GitGraph actions"))
}

fn run_gitgraph_action(
    repo_root: &str,
    args: Vec<String>,
) -> Result<zed::SlashCommandOutput, String> {
    let parsed = parse_action_args(args)?;
    let catalog = ActionCatalog::with_defaults();
    let request = ActionRequest {
        template_id: parsed.template_id.clone(),
        params: parsed.params.clone(),
        enabled_options: parsed.enabled_options.clone(),
        context: parsed.context.clone(),
    };
    let resolved = catalog
        .resolve_with_lookup(request, |placeholder| {
            lookup_dynamic_placeholder(repo_root, placeholder)
        })
        .map_err(|e| format!("resolve action failed: {e}"))?;

    let output = if let Some(script) = &resolved.shell_script {
        run_shell_command(repo_root, &format!("git {}", script))?
    } else {
        run_git_command(repo_root, &resolved.args)?
    };
    let status = output.status.unwrap_or(-1);
    if status != 0 && !resolved.allow_non_zero_exit && !resolved.ignore_errors {
        return Err(format!(
            "git action failed (exit {status}):\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let mut text = String::new();
    text.push_str(&format!("# GitGraph action: `{}`\n\n", resolved.id));
    text.push_str(&format!("Command: `git {}`\n", resolved.command_line));
    text.push_str(&format!("Exit: `{}`\n\n", status));
    if !output.stdout.is_empty() {
        text.push_str("## stdout\n");
        text.push_str("```text\n");
        text.push_str(&String::from_utf8_lossy(&output.stdout));
        text.push_str("\n```\n");
    }
    if !output.stderr.is_empty() {
        text.push_str("## stderr\n");
        text.push_str("```text\n");
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        text.push_str("\n```\n");
    }
    if output.stdout.is_empty() && output.stderr.is_empty() {
        text.push_str("(no output)\n");
    }
    Ok(build_output(text, "GitGraph action"))
}

fn run_gitgraph_blame(
    repo_root: &str,
    args: Vec<String>,
) -> Result<zed::SlashCommandOutput, String> {
    let (file, line) = parse_blame_args(args)?;
    let out = run_git_command(
        repo_root,
        &[
            "blame".to_string(),
            format!("-L{line},{line}"),
            "--porcelain".to_string(),
            "--".to_string(),
            file.clone(),
        ],
    )?;
    if out.status != Some(0) {
        return Err(format!(
            "git blame failed (exit {:?}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = render_blame_text(
        repo_root,
        &file,
        line,
        &String::from_utf8_lossy(&out.stdout),
    );
    Ok(build_output(text, "GitGraph blame"))
}

fn run_gitgraph_tips() -> Result<zed::SlashCommandOutput, String> {
    let text = [
        "# GitGraph tips",
        "",
        "- `/gitgraph-log [limit]` - show recent graph summary",
        "- `/gitgraph-search [limit=200] [path=src/file.rs] query` - search history",
        "- `/gitgraph-actions` - list action ids",
        "- `/gitgraph-action <id> KEY=VALUE +opt:<option-id>` - run action",
        "- `/gitgraph-blame <path> <line>` - single-line blame",
        "",
        "For full-screen interactive graph use CLI TUI in terminal:",
        "`gitgraph`",
    ]
    .join("\n");
    Ok(build_output(text, "GitGraph tips"))
}

fn parse_limit_arg(arg: Option<&str>) -> Result<usize, String> {
    match arg {
        None => Ok(DEFAULT_LIMIT),
        Some(raw) if raw.trim().is_empty() => Ok(DEFAULT_LIMIT),
        Some(raw) => {
            let raw = raw.trim();
            let value = if let Some((_, rhs)) = raw.split_once("limit=") {
                rhs
            } else {
                raw
            };
            let n = value
                .parse::<usize>()
                .map_err(|e| format!("invalid limit {:?}: {}", raw, e))?;
            if n == 0 || n > MAX_LIMIT {
                return Err(format!("limit must be between 1 and {}", MAX_LIMIT));
            }
            Ok(n)
        }
    }
}

fn parse_search_args(args: Vec<String>) -> Result<ParsedSearchArgs, String> {
    if args.is_empty() {
        return Err("usage: /gitgraph-search [limit=200] [path=src/file.rs] <query>".to_string());
    }
    let mut limit: Option<usize> = None;
    let mut file_path = None;
    let mut query_parts = Vec::new();
    for arg in args {
        if limit.is_none() && (arg.starts_with("limit=") || arg.chars().all(|c| c.is_ascii_digit()))
        {
            limit = Some(parse_limit_arg(Some(&arg))?);
            continue;
        }
        if let Some(path) = arg.strip_prefix("path=") {
            let path = path.trim();
            if path.is_empty() {
                return Err("path=... value must not be empty".to_string());
            }
            file_path = Some(path.replace('\\', "/"));
            continue;
        }
        query_parts.push(arg);
    }
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        return Err("usage: /gitgraph-search [limit=200] [path=src/file.rs] <query>".to_string());
    }
    Ok(ParsedSearchArgs {
        limit: limit.unwrap_or(DEFAULT_LIMIT),
        file_path,
        query,
    })
}

#[derive(Debug)]
struct ParsedSearchArgs {
    limit: usize,
    file_path: Option<String>,
    query: String,
}

fn parse_blame_args(args: Vec<String>) -> Result<(String, usize), String> {
    if args.len() < 2 {
        return Err("usage: /gitgraph-blame <path> <line>".to_string());
    }
    let file = args[0].clone();
    let line = args[1]
        .parse::<usize>()
        .map_err(|e| format!("invalid line {:?}: {}", args[1], e))?;
    if line == 0 {
        return Err("line must be >= 1".to_string());
    }
    Ok((file, line))
}

#[derive(Debug)]
struct ParsedActionArgs {
    template_id: String,
    params: HashMap<String, String>,
    enabled_options: HashSet<String>,
    context: ActionContext,
}

fn parse_action_args(args: Vec<String>) -> Result<ParsedActionArgs, String> {
    let Some((template_id, tail)) = args.split_first() else {
        return Err(
            "usage: /gitgraph-action <action-id> KEY=VALUE +opt:<option-id> (e.g. BRANCH_NAME=main)"
                .to_string(),
        );
    };
    let mut params = HashMap::new();
    let mut enabled_options = HashSet::new();
    let mut context = ActionContext::default();
    context.default_remote_name = Some("origin".to_string());

    for token in tail {
        if let Some(opt) = token.strip_prefix("+opt:") {
            enabled_options.insert(opt.to_string());
            continue;
        }
        let (key, value) = token
            .split_once('=')
            .ok_or_else(|| format!("invalid token {:?}, expected KEY=VALUE or +opt:<id>", token))?;
        params.insert(key.to_string(), value.to_string());
        map_context_placeholder(&mut context, key, value);
    }

    Ok(ParsedActionArgs {
        template_id: template_id.to_string(),
        params,
        enabled_options,
        context,
    })
}

fn map_context_placeholder(context: &mut ActionContext, key: &str, value: &str) {
    match key {
        "BRANCH_DISPLAY_NAME" => context.branch_display_name = Some(value.to_string()),
        "BRANCH_NAME" => context.branch_name = Some(value.to_string()),
        "LOCAL_BRANCH_NAME" => context.local_branch_name = Some(value.to_string()),
        "BRANCH_ID" => context.branch_id = Some(value.to_string()),
        "SOURCE_BRANCH_NAME" => context.source_branch_name = Some(value.to_string()),
        "TARGET_BRANCH_NAME" => context.target_branch_name = Some(value.to_string()),
        "COMMIT_HASH" => context.commit_hash = Some(value.to_string()),
        "COMMIT_HASHES" => {
            context.commit_hashes = value
                .split([',', ' '])
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect()
        }
        "COMMIT_BODY" => context.commit_body = Some(value.to_string()),
        "STASH_NAME" => context.stash_name = Some(value.to_string()),
        "TAG_NAME" => context.tag_name = Some(value.to_string()),
        "REMOTE_NAME" => context.remote_name = Some(value.to_string()),
        "DEFAULT_REMOTE_NAME" => context.default_remote_name = Some(value.to_string()),
        _ => {
            context
                .additional_placeholders
                .insert(key.to_string(), value.to_string());
        }
    }
}

fn lookup_dynamic_placeholder(
    repo_root: &str,
    placeholder: &str,
) -> gitgraph_core::Result<Option<String>> {
    if let Some(key) = placeholder.strip_prefix("GIT_CONFIG:") {
        let out = run_git_command(
            repo_root,
            &["config".to_string(), "--get".to_string(), key.to_string()],
        )
        .map_err(GitLgError::State)?;
        return Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ));
    }
    if let Some(raw_exec) = placeholder.strip_prefix("GIT_EXEC:") {
        let args = shlex::split(raw_exec).unwrap_or_else(|| {
            raw_exec
                .split_whitespace()
                .map(ToString::to_string)
                .collect()
        });
        let out = run_git_command(repo_root, &args).map_err(GitLgError::State)?;
        return Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ));
    }
    Ok(None)
}

fn run_git_log(repo_root: &str, limit: usize) -> Result<String, String> {
    let args = vec![
        "-c".to_string(),
        "color.ui=never".to_string(),
        "log".to_string(),
        "--date-order".to_string(),
        "--topo-order".to_string(),
        "--decorate=full".to_string(),
        "--color=never".to_string(),
        "--no-show-signature".to_string(),
        "--no-notes".to_string(),
        "--all".to_string(),
        "-n".to_string(),
        limit.to_string(),
        "--pretty=format:%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%ct%x1f%D%x1f%s%x1f%b%x1e"
            .to_string(),
    ];
    let out = run_git_command(repo_root, &args)?;
    if out.status != Some(0) {
        return Err(format!(
            "git log failed (exit {:?}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("invalid utf-8 from git: {}", e))
}

fn run_git_command(repo_root: &str, args: &[String]) -> Result<zed::process::Output, String> {
    let mut full_args = vec!["-C".to_string(), repo_root.to_string()];
    full_args.extend(args.to_vec());
    let mut cmd = zed::process::Command::new("git").args(full_args);
    cmd.output()
}

fn run_shell_command(repo_root: &str, script: &str) -> Result<zed::process::Output, String> {
    let (os, _arch) = zed::current_platform();
    match os {
        zed::Os::Windows => {
            let mut cmd = zed::process::Command::new("cmd").args([
                "/C".to_string(),
                format!("cd /d \"{}\" && {}", repo_root, script),
            ]);
            cmd.output()
        }
        _ => {
            let mut cmd = zed::process::Command::new("sh").args([
                "-lc".to_string(),
                format!("cd \"{}\" && {}", repo_root, script),
            ]);
            cmd.output()
        }
    }
}

fn render_rows(repo_root: &str, rows: &[gitgraph_core::GraphRow], subtitle: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("# GitGraph log for `{}`\n\n", repo_root));
    out.push_str(subtitle);
    out.push_str("\n\n");

    for row in rows {
        let graph_prefix = format!("{}*", "| ".repeat(row.lane));
        let refs = if row.refs.is_empty() {
            String::new()
        } else {
            let names = row
                .refs
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(" ({})", names)
        };
        out.push_str(&format!(
            "- {} `{}` {}{} - {}\n",
            graph_prefix, row.short_hash, row.subject, refs, row.author_name
        ));
    }

    if rows.is_empty() {
        out.push_str("- (no commits found for current selection)\n");
    }
    out
}

fn render_blame_text(repo_root: &str, file: &str, line: usize, raw: &str) -> String {
    let mut commit_hash = "";
    let mut author = "";
    let mut author_mail = "";
    let mut summary = "";
    let mut author_time = "";
    for (idx, l) in raw.lines().enumerate() {
        if idx == 0 {
            commit_hash = l.split_whitespace().next().unwrap_or_default();
            continue;
        }
        if let Some(v) = l.strip_prefix("author ") {
            author = v;
            continue;
        }
        if let Some(v) = l.strip_prefix("author-mail ") {
            author_mail = v;
            continue;
        }
        if let Some(v) = l.strip_prefix("author-time ") {
            author_time = v;
            continue;
        }
        if let Some(v) = l.strip_prefix("summary ") {
            summary = v;
            continue;
        }
    }
    [
        format!("# GitGraph blame for `{}`", repo_root),
        String::new(),
        format!("file: `{}`", file),
        format!("line: `{}`", line),
        format!("commit: `{}`", commit_hash),
        format!("author: `{}` {}", author, author_mail),
        format!("author_time_unix: `{}`", author_time),
        format!("summary: {}", summary),
    ]
    .join("\n")
}

fn build_output(text: String, label: &str) -> zed::SlashCommandOutput {
    let end = text.len() as u32;
    zed::SlashCommandOutput {
        text,
        sections: vec![zed::SlashCommandOutputSection {
            range: zed::Range { start: 0, end },
            label: label.to_string(),
        }],
    }
}

zed::register_extension!(GitGraphZedExtension);
