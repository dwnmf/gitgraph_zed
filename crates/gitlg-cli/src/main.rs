use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::responses::{
    CreateResponse, CreateResponseArgs, Reasoning, ReasoningEffort, ResponseStreamEvent,
};
use clap::{Args, Parser, Subcommand};
use gitlg_core::{
    ActionContext, ActionRequest, CommitSearchQuery, GitLgService, GitOutput, GitRunner,
    GraphQuery, StateStore,
};
use serde::Deserialize;
use tokio_stream::StreamExt;

mod tui;

#[derive(Debug, Parser)]
#[command(name = "gitgraph")]
#[command(about = "GitGraph Rust CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Graph(GraphCmd),
    Tui(TuiCmd),
    Search(SearchCmd),
    Blame(BlameCmd),
    CommitDesc(CommitDescCmd),
    Actions(ActionsCmd),
    State(StateCmd),
    ValidateRepo(RepoCmd),
}

#[derive(Debug, Args)]
struct RepoCmd {
    #[arg(long)]
    repo: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct GraphCmd {
    #[arg(long)]
    repo: Option<PathBuf>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    skip: Option<usize>,
    #[arg(long)]
    all: bool,
    #[arg(long)]
    no_stash: bool,
    #[arg(long = "arg")]
    arg: Vec<String>,
    #[arg(long)]
    pretty: bool,
}

#[derive(Debug, Args)]
struct TuiCmd {
    #[arg(long)]
    repo: Option<PathBuf>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    skip: Option<usize>,
    #[arg(long)]
    all: bool,
    #[arg(long)]
    no_stash: bool,
    #[arg(long = "arg")]
    arg: Vec<String>,
    #[arg(long, value_enum, default_value_t = tui::GraphStyle::Ascii)]
    graph_style: tui::GraphStyle,
    #[arg(long, default_value_t = 0)]
    max_patch_lines: usize,
}

impl Default for TuiCmd {
    fn default() -> Self {
        Self {
            repo: None,
            limit: None,
            skip: None,
            all: false,
            no_stash: false,
            arg: Vec::new(),
            graph_style: tui::GraphStyle::Ascii,
            max_patch_lines: 0,
        }
    }
}

#[derive(Debug, Args)]
struct SearchCmd {
    #[arg(long)]
    repo: Option<PathBuf>,
    #[arg(long)]
    text: String,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    skip: Option<usize>,
    #[arg(long)]
    regex: bool,
    #[arg(long)]
    case_sensitive: bool,
    #[arg(long)]
    pretty: bool,
}

#[derive(Debug, Args)]
struct BlameCmd {
    #[arg(long)]
    repo: Option<PathBuf>,
    #[arg(long)]
    file: PathBuf,
    #[arg(long)]
    line: usize,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningEffortArg {
    Minimal,
    Low,
    Medium,
    High,
}

#[derive(Debug, Args)]
struct CommitDescCmd {
    #[arg(long)]
    repo: Option<PathBuf>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, value_enum)]
    reasoning_effort: Option<ReasoningEffortArg>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    chatgpt_base_url: Option<String>,
    #[arg(long)]
    requires_openai_auth: Option<bool>,
    #[arg(long)]
    codex_auth_json: Option<PathBuf>,
    #[arg(long)]
    codex_auth_token_env: Option<String>,
    #[arg(long = "wire-api")]
    wire_api: Option<String>,
    #[arg(long)]
    max_output_tokens: Option<u32>,
    #[arg(long)]
    max_diff_chars: Option<usize>,
}

#[derive(Debug, Subcommand)]
enum ActionsSubcommand {
    List,
    Run(RunActionCmd),
    Preview(RunActionCmd),
}

#[derive(Debug, Args)]
struct ActionsCmd {
    #[command(subcommand)]
    subcommand: ActionsSubcommand,
}

#[derive(Debug, Args)]
struct RunActionCmd {
    #[arg(long)]
    repo: Option<PathBuf>,
    #[arg(long)]
    id: String,
    #[arg(long = "param", value_name = "KEY=VALUE")]
    params: Vec<String>,
    #[arg(long = "option")]
    options: Vec<String>,
    #[arg(long = "ctx", value_name = "KEY=VALUE")]
    ctx: Vec<String>,
    #[arg(long)]
    context_json: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum StateSubcommand {
    Show,
    SetRepo { path: PathBuf },
    SetGitBinary { binary: String },
}

#[derive(Debug, Args)]
struct StateCmd {
    #[command(subcommand)]
    subcommand: StateSubcommand,
}

const COMMIT_DESC_INSTRUCTIONS: &str = "You write git commit descriptions based on provided changes.\n\
Return plain text only.\n\
Format exactly:\n\
1) First line: concise imperative commit subject (max 72 chars)\n\
2) Blank line\n\
3) 2-6 bullet points starting with '- ' summarizing key changes and user-visible impact.\n\
Do not use markdown headings, code fences, or backticks.";
const LOCAL_CONFIG_FILE: &str = ".config.toml";
const DEFAULT_COMMIT_DESC_MODEL: &str = "gpt-5-mini";
const DEFAULT_COMMIT_DESC_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_COMMIT_DESC_MAX_OUTPUT_TOKENS: u32 = 1200;
const DEFAULT_COMMIT_DESC_MAX_DIFF_CHARS: usize = 120_000;
const DEFAULT_API_KEY_ENV: &str = "OPENAI_API_KEY";
const DEFAULT_CODEX_AUTH_TOKEN_ENV: &str = "OPENAI_ACCESS_TOKEN";
const DEFAULT_CODEX_AUTH_PLACEHOLDER: &str = "codex-auth";
const DEFAULT_WIRE_API: &str = "responses";
const DEFAULT_GITGRAPH_CONFIG_TEMPLATE: &str = "# Auto-generated by gitgraph.\n\
\n\
[gitgraph.openai]\n\
# Defaults for `gitgraph commit-desc`\n\
model = \"gpt-5-mini\"\n\
reasoning_effort = \"medium\"\n\
base_url = \"https://api.openai.com/v1\"\n\
api_key_env = \"OPENAI_API_KEY\"\n\
wire_api = \"responses\"\n\
max_output_tokens = 1200\n\
max_diff_chars = 120000\n\
\n\
# codex-lb compatibility (optional):\n\
# When true, codex auth mode is used and API key mode is ignored.\n\
requires_openai_auth = false\n\
codex_auth_json = \"~/.codex/auth.json\"\n\
codex_auth_token_env = \"OPENAI_ACCESS_TOKEN\"\n\
# chatgpt_base_url = \"http://127.0.0.1:2455\"\n\
\n\
# Optional direct key (not recommended):\n\
# api_key = \"sk-...\"\n";

#[derive(Debug)]
struct WorkingTreeChanges {
    status_short: String,
    staged_diff: String,
    unstaged_diff: String,
}

#[derive(Debug, Default, Deserialize)]
struct LocalConfigToml {
    #[serde(default)]
    gitgraph: GitGraphConfigSection,
}

#[derive(Debug, Default, Deserialize)]
struct GitGraphConfigSection {
    #[serde(default)]
    openai: GitGraphOpenAiConfig,
}

#[derive(Debug, Default, Deserialize)]
struct GitGraphOpenAiConfig {
    model: Option<String>,
    reasoning_effort: Option<ReasoningEffortArg>,
    base_url: Option<String>,
    api_key: Option<String>,
    chatgpt_base_url: Option<String>,
    requires_openai_auth: Option<bool>,
    codex_auth_json: Option<String>,
    codex_auth_token_env: Option<String>,
    api_key_env: Option<String>,
    wire_api: Option<String>,
    max_output_tokens: Option<u32>,
    max_diff_chars: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct CodexAuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<CodexAuthTokens>,
}

#[derive(Debug, Deserialize)]
struct CodexAuthTokens {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "idToken")]
    id_token: Option<String>,
}

#[derive(Debug, Clone)]
struct CommitDescSettings {
    model: String,
    reasoning_effort: Option<ReasoningEffortArg>,
    base_url: Option<String>,
    api_key: Option<String>,
    chatgpt_base_url: Option<String>,
    requires_openai_auth: bool,
    codex_auth_json: Option<PathBuf>,
    codex_auth_token_env: String,
    api_key_env: String,
    wire_api: Option<String>,
    max_output_tokens: u32,
    max_diff_chars: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli)
}

fn run(cli: Cli) -> Result<()> {
    let store = StateStore::default_store().context("failed to resolve state store path")?;
    let mut state = store.load().context("failed to load state")?;

    let runner = GitRunner::new(
        state
            .preferred_git_binary
            .clone()
            .unwrap_or_else(|| "git".to_string()),
    );
    let service = GitLgService::new(runner.clone(), state.actions.clone());

    let command = cli.command.unwrap_or(Commands::Tui(TuiCmd::default()));

    match command {
        Commands::Graph(cmd) => {
            let repo = resolve_repo(cmd.repo)?;
            let query = build_query_from_graph_args(
                state.graph_query.clone(),
                cmd.limit,
                cmd.skip,
                cmd.all,
                cmd.no_stash,
                cmd.arg,
            );
            let graph = service
                .graph(&repo, &query)
                .with_context(|| format!("failed to load graph for {}", repo.display()))?;
            let json = if cmd.pretty {
                serde_json::to_string_pretty(&graph)?
            } else {
                serde_json::to_string(&graph)?
            };
            println!("{json}");
        }
        Commands::Tui(cmd) => {
            let repo = resolve_repo(cmd.repo)?;
            let query = build_query_from_graph_args(
                state.graph_query.clone(),
                cmd.limit,
                cmd.skip,
                cmd.all,
                cmd.no_stash,
                cmd.arg,
            );
            tui::run(
                &service,
                tui::TuiConfig {
                    repo: repo.clone(),
                    query,
                    graph_style: cmd.graph_style,
                    max_patch_lines: cmd.max_patch_lines,
                    git_binary: runner.git_binary().to_string(),
                },
            )
            .with_context(|| format!("failed running TUI for {}", repo.display()))?;
        }
        Commands::Search(cmd) => {
            let repo = resolve_repo(cmd.repo)?;
            let mut query = state.graph_query.clone();
            if let Some(limit) = cmd.limit {
                query.limit = limit;
            }
            if let Some(skip) = cmd.skip {
                query.skip = skip;
            }
            let search = CommitSearchQuery {
                text: cmd.text,
                file_path: cmd.file.map(path_to_git_path),
                case_sensitive: cmd.case_sensitive,
                use_regex: cmd.regex,
                ..CommitSearchQuery::default()
            };
            let graph = service
                .graph_filtered(&repo, &query, &search)
                .with_context(|| format!("failed to search graph for {}", repo.display()))?;
            let json = if cmd.pretty {
                serde_json::to_string_pretty(&graph)?
            } else {
                serde_json::to_string(&graph)?
            };
            println!("{json}");
        }
        Commands::Blame(cmd) => {
            let repo = resolve_repo(cmd.repo)?;
            let blame = service
                .blame_line(&repo, &cmd.file, cmd.line)
                .with_context(|| format!("failed to blame {}:{}", cmd.file.display(), cmd.line))?;
            println!("{}", serde_json::to_string_pretty(&blame)?);
        }
        Commands::CommitDesc(cmd) => {
            let repo = resolve_repo(cmd.repo.clone())?;
            let openai_cfg = load_or_create_gitgraph_openai_config(&repo)?;
            let settings = resolve_commit_desc_settings(&cmd, &openai_cfg);
            let changes = collect_uncommitted_changes(&runner, &repo)?;
            let prompt = build_commit_description_prompt(&changes, settings.max_diff_chars);
            let generated = generate_commit_description_with_openai(&settings, prompt)?;
            println!("{}", generated.trim());
        }
        Commands::Actions(cmd) => match cmd.subcommand {
            ActionsSubcommand::List => {
                println!("{}", serde_json::to_string_pretty(service.actions())?);
            }
            ActionsSubcommand::Preview(run) => {
                let request = build_action_request(run)?;
                let resolved =
                    service.resolve_action_preview(request, &state.default_remote_name, None)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": resolved.id,
                        "scope": resolved.scope,
                        "args": resolved.args,
                        "shell_script": resolved.shell_script,
                        "command_line": resolved.command_line,
                    }))?
                );
            }
            ActionsSubcommand::Run(run) => {
                let repo = resolve_repo(run.repo.clone())?;
                let request = build_action_request(run)?;
                let result = service.execute_action(&repo, request, &state.default_remote_name)?;
                eprintln!("executed: git {}", result.command_line);
                println!("{}", result.output.stdout);
                if !result.output.stderr.trim().is_empty() {
                    eprintln!("{}", result.output.stderr);
                }
            }
        },
        Commands::State(cmd) => match cmd.subcommand {
            StateSubcommand::Show => {
                println!("{}", serde_json::to_string_pretty(&state)?);
            }
            StateSubcommand::SetRepo { path } => {
                state.selected_repo_path = Some(path);
                store.save(&state).context("failed to save state")?;
                println!("selected repo updated");
            }
            StateSubcommand::SetGitBinary { binary } => {
                state.preferred_git_binary = Some(binary);
                store.save(&state).context("failed to save state")?;
                println!("preferred git binary updated");
            }
        },
        Commands::ValidateRepo(cmd) => {
            let repo = resolve_repo(cmd.repo)?;
            service
                .graph(
                    &repo,
                    &GraphQuery {
                        limit: 1,
                        skip: 0,
                        all_refs: true,
                        include_stash_ref: false,
                        additional_args: vec![],
                    },
                )
                .with_context(|| format!("repo validation failed for {}", repo.display()))?;
            println!("ok");
        }
    }

    Ok(())
}

fn resolve_repo(cli_repo: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(repo) = cli_repo {
        return Ok(repo);
    }

    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    if cwd.join(".git").exists() {
        return Ok(cwd);
    }

    Err(anyhow!(
        "current directory is not a git repository (pass --repo, or run `gitgraph` inside a git repo)"
    ))
}

fn build_action_request(cmd: RunActionCmd) -> Result<ActionRequest> {
    let params = parse_key_value_args(cmd.params)?;
    let enabled_options = cmd.options.into_iter().collect::<HashSet<_>>();
    let mut context = ActionContext::default();
    if let Some(path) = cmd.context_json {
        let json = fs::read_to_string(&path)
            .with_context(|| format!("failed to read context json from {}", path.display()))?;
        context = serde_json::from_str(&json).with_context(|| {
            format!(
                "failed to deserialize ActionContext from {}",
                path.display()
            )
        })?;
    }
    apply_context_pairs(&mut context, cmd.ctx)?;
    Ok(ActionRequest {
        template_id: cmd.id,
        params,
        enabled_options,
        context,
    })
}

fn parse_key_value_args(args: Vec<String>) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for pair in args {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("invalid --param value {:?}, expected KEY=VALUE", pair))?;
        out.insert(key.to_string(), value.to_string());
    }
    Ok(out)
}

fn parse_key_value_pair(pair: &str) -> Result<(String, String)> {
    let (key, value) = pair
        .split_once('=')
        .ok_or_else(|| anyhow!("invalid value {:?}, expected KEY=VALUE", pair))?;
    Ok((key.to_string(), value.to_string()))
}

fn apply_context_pairs(context: &mut ActionContext, pairs: Vec<String>) -> Result<()> {
    for pair in pairs {
        let (key, value) = parse_key_value_pair(&pair)?;
        apply_context_placeholder(context, &key, &value);
    }
    Ok(())
}

fn apply_context_placeholder(context: &mut ActionContext, key: &str, value: &str) {
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

fn build_query_from_graph_args(
    mut base: GraphQuery,
    limit: Option<usize>,
    skip: Option<usize>,
    all: bool,
    no_stash: bool,
    additional_args: Vec<String>,
) -> GraphQuery {
    if let Some(limit) = limit {
        base.limit = limit;
    }
    if let Some(skip) = skip {
        base.skip = skip;
    }
    if all {
        base.all_refs = true;
    }
    if no_stash {
        base.include_stash_ref = false;
    }
    if !additional_args.is_empty() {
        base.additional_args = additional_args;
    }
    base
}

fn path_to_git_path(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn ensure_gitgraph_local_config(repo: &Path) -> Result<()> {
    ensure_gitgraph_local_config_file(repo)?;
    Ok(())
}

pub(crate) fn generate_commit_description_for_repo_with_git(
    repo: &Path,
    git_binary: &str,
) -> Result<String> {
    let runner = GitRunner::new(git_binary.to_string());
    let cfg = load_or_create_gitgraph_openai_config(repo)?;
    let defaults = CommitDescCmd {
        repo: None,
        model: None,
        reasoning_effort: None,
        base_url: None,
        api_key: None,
        chatgpt_base_url: None,
        requires_openai_auth: None,
        codex_auth_json: None,
        codex_auth_token_env: None,
        wire_api: None,
        max_output_tokens: None,
        max_diff_chars: None,
    };
    let settings = resolve_commit_desc_settings(&defaults, &cfg);
    let changes = collect_uncommitted_changes(&runner, repo)?;
    let prompt = build_commit_description_prompt(&changes, settings.max_diff_chars);
    generate_commit_description_with_openai(&settings, prompt)
}

pub(crate) fn auto_commit_with_message(
    repo: &Path,
    git_binary: &str,
    message: &str,
) -> Result<String> {
    let runner = GitRunner::new(git_binary.to_string());
    runner.validate_repo(repo)?;

    let (subject, body) = split_commit_message(message)?;
    runner.exec(repo, &["add".to_string(), "-A".to_string()], false)?;

    let mut args = vec!["commit".to_string(), "-m".to_string(), subject];
    if !body.is_empty() {
        args.push("-m".to_string());
        args.push(body);
    }
    let out = runner.exec(repo, &args, false)?;
    Ok(summarize_git_output(&out))
}

pub(crate) fn auto_push_current_branch(repo: &Path, git_binary: &str) -> Result<String> {
    let runner = GitRunner::new(git_binary.to_string());
    runner.validate_repo(repo)?;
    let out = runner.exec(repo, &["push".to_string()], false)?;
    Ok(summarize_git_output(&out))
}

fn load_or_create_gitgraph_openai_config(repo: &Path) -> Result<GitGraphOpenAiConfig> {
    let path = ensure_gitgraph_local_config_file(repo)?;
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    match toml::from_str::<LocalConfigToml>(&raw) {
        Ok(parsed) => Ok(parsed.gitgraph.openai),
        Err(err) => {
            eprintln!(
                "warning: failed to parse {}, using defaults: {}",
                path.display(),
                err
            );
            Ok(GitGraphOpenAiConfig::default())
        }
    }
}

fn ensure_gitgraph_local_config_file(repo: &Path) -> Result<PathBuf> {
    let path = repo.join(LOCAL_CONFIG_FILE);
    if !path.exists() {
        fs::write(&path, DEFAULT_GITGRAPH_CONFIG_TEMPLATE)
            .with_context(|| format!("failed to create {}", path.display()))?;
        eprintln!("created {}", path.display());
    }
    Ok(path)
}

fn resolve_commit_desc_settings(
    cmd: &CommitDescCmd,
    cfg: &GitGraphOpenAiConfig,
) -> CommitDescSettings {
    let codex_auth_token_env = cmd
        .codex_auth_token_env
        .clone()
        .or_else(|| cfg.codex_auth_token_env.clone())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CODEX_AUTH_TOKEN_ENV.to_string());
    let api_key_env = cfg
        .api_key_env
        .clone()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_API_KEY_ENV.to_string());

    CommitDescSettings {
        model: cmd
            .model
            .clone()
            .or_else(|| cfg.model.clone())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_COMMIT_DESC_MODEL.to_string()),
        reasoning_effort: cmd.reasoning_effort.or(cfg.reasoning_effort),
        base_url: cmd
            .base_url
            .clone()
            .or_else(|| cfg.base_url.clone())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        api_key: cmd
            .api_key
            .clone()
            .or_else(|| cfg.api_key.clone())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        chatgpt_base_url: cmd
            .chatgpt_base_url
            .clone()
            .or_else(|| cfg.chatgpt_base_url.clone())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        requires_openai_auth: cmd
            .requires_openai_auth
            .or(cfg.requires_openai_auth)
            .unwrap_or(false),
        codex_auth_json: cmd
            .codex_auth_json
            .clone()
            .or_else(|| cfg.codex_auth_json.clone().map(PathBuf::from))
            .and_then(normalize_path_setting),
        codex_auth_token_env,
        api_key_env,
        wire_api: cmd
            .wire_api
            .clone()
            .or_else(|| cfg.wire_api.clone())
            .or_else(|| Some(DEFAULT_WIRE_API.to_string()))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        max_output_tokens: cmd
            .max_output_tokens
            .or(cfg.max_output_tokens)
            .unwrap_or(DEFAULT_COMMIT_DESC_MAX_OUTPUT_TOKENS),
        max_diff_chars: cmd
            .max_diff_chars
            .or(cfg.max_diff_chars)
            .unwrap_or(DEFAULT_COMMIT_DESC_MAX_DIFF_CHARS),
    }
}

fn split_commit_message(message: &str) -> Result<(String, String)> {
    let mut lines = message.lines().skip_while(|line| line.trim().is_empty());
    let subject = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| anyhow!("generated commit description is empty"))?
        .to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Ok((subject, body))
}

fn summarize_git_output(out: &GitOutput) -> String {
    let stdout = out.stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    let stderr = out.stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    "ok".to_string()
}

fn collect_uncommitted_changes(runner: &GitRunner, repo: &Path) -> Result<WorkingTreeChanges> {
    runner.validate_repo(repo)?;
    let raw_status = run_git_stdout(
        runner,
        repo,
        &[
            "status".to_string(),
            "--short".to_string(),
            "--untracked-files=all".to_string(),
        ],
        false,
    )?;
    let status_short = filter_status_for_commit_desc(&raw_status);
    if status_short.trim().is_empty() {
        return Err(anyhow!(
            "working tree is clean (or only .config.toml changed): no commit changes found"
        ));
    }

    let staged_diff = run_git_stdout(
        runner,
        repo,
        &[
            "diff".to_string(),
            "--staged".to_string(),
            "--no-color".to_string(),
            "--no-ext-diff".to_string(),
        ],
        false,
    )?;
    let unstaged_diff = run_git_stdout(
        runner,
        repo,
        &[
            "diff".to_string(),
            "--no-color".to_string(),
            "--no-ext-diff".to_string(),
        ],
        false,
    )?;

    Ok(WorkingTreeChanges {
        status_short,
        staged_diff,
        unstaged_diff,
    })
}

fn filter_status_for_commit_desc(status_short: &str) -> String {
    status_short
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && trimmed != "?? .config.toml"
                && trimmed != "?? ./.config.toml"
                && trimmed != "A  .config.toml"
                && trimmed != " M .config.toml"
                && trimmed != "M  .config.toml"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_git_stdout(
    runner: &GitRunner,
    repo: &Path,
    args: &[String],
    allow_non_zero: bool,
) -> Result<String> {
    Ok(runner.exec(repo, args, allow_non_zero)?.stdout)
}

fn build_commit_description_prompt(changes: &WorkingTreeChanges, max_diff_chars: usize) -> String {
    let bounded = max_diff_chars.max(1_000);
    let staged_budget = bounded / 2;
    let unstaged_budget = bounded.saturating_sub(staged_budget);

    let staged = truncate_for_prompt(&changes.staged_diff, staged_budget);
    let unstaged = truncate_for_prompt(&changes.unstaged_diff, unstaged_budget);

    format!(
        "Generate a commit description for the following uncommitted git changes.\n\
Output language: Russian.\n\n\
## git status --short\n\
{}\n\n\
## Staged diff (git diff --staged)\n\
{}\n\n\
## Unstaged diff (git diff)\n\
{}\n",
        if changes.status_short.trim().is_empty() {
            "(empty)"
        } else {
            changes.status_short.trim()
        },
        if staged.trim().is_empty() {
            "(empty)"
        } else {
            staged.trim()
        },
        if unstaged.trim().is_empty() {
            "(empty)"
        } else {
            unstaged.trim()
        }
    )
}

fn truncate_for_prompt(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let total = input.chars().count();
    if total <= max_chars {
        return input.to_string();
    }
    let mut clipped = input.chars().take(max_chars).collect::<String>();
    clipped.push_str(&format!(
        "\n... [truncated {} chars]",
        total.saturating_sub(max_chars)
    ));
    clipped
}

fn generate_commit_description_with_openai(
    settings: &CommitDescSettings,
    prompt: String,
) -> Result<String> {
    let api_key_hint = if settings.requires_openai_auth {
        format!(
            "{}, OPENAI_ACCESS_TOKEN, CODEX_ACCESS_TOKEN, or codex auth.json",
            settings.codex_auth_token_env
        )
    } else if settings.api_key_env == DEFAULT_API_KEY_ENV {
        DEFAULT_API_KEY_ENV.to_string()
    } else {
        format!("{}/{}", settings.api_key_env, DEFAULT_API_KEY_ENV)
    };
    let api_key = resolve_api_key(settings).with_context(|| {
        format!(
            "OpenAI auth value is missing for active mode (set {})",
            api_key_hint
        )
    })?;

    let mut config = OpenAIConfig::new().with_api_key(api_key);
    if let Some(base_url) = settings
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        config = config.with_api_base(base_url);
    } else {
        config = config.with_api_base(DEFAULT_COMMIT_DESC_BASE_URL);
    }
    if let Some(wire_api) = settings
        .wire_api
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        config = config
            .with_header("wire-api", wire_api)
            .map_err(|e| anyhow!("failed to set wire-api header: {e}"))?;
    }
    if let Some(chatgpt_base_url) = settings
        .chatgpt_base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        config = config
            .with_header("chatgpt-base-url", chatgpt_base_url)
            .map_err(|e| anyhow!("failed to set chatgpt-base-url header: {e}"))?;
    }
    if settings.requires_openai_auth {
        config = config
            .with_header("requires-openai-auth", "true")
            .map_err(|e| anyhow!("failed to set requires-openai-auth header: {e}"))?;
    }
    let client = Client::with_config(config);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to initialize async runtime for OpenAI request")?;

    runtime.block_on(async {
        let mut request = CreateResponseArgs::default();
        request.model(settings.model.clone());
        request.instructions(COMMIT_DESC_INSTRUCTIONS.to_string());
        request.input(vec![prompt]);
        request.max_output_tokens(settings.max_output_tokens);
        if let Some(effort) = settings.reasoning_effort {
            request.reasoning(Reasoning {
                effort: Some(map_reasoning_effort(effort)),
                summary: None,
            });
        }
        let request: CreateResponse = request
            .build()
            .map_err(|e| anyhow!("failed to build OpenAI request: {e}"))?;

        match client.responses().create(request.clone()).await {
            Ok(response) => response
                .output_text()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("OpenAI returned no text output")),
            Err(err) => {
                let err_text = err.to_string();
                if requires_stream_mode(&err_text) {
                    return generate_commit_description_streaming(&client, request).await;
                }
                Err(anyhow!("OpenAI request failed: {err_text}"))
            }
        }
    })
}

fn requires_stream_mode(error_text: &str) -> bool {
    let msg = error_text.to_lowercase();
    msg.contains("stream must be set to true")
        || msg.contains("stream=true")
        || msg.contains("upstream_error")
}

async fn generate_commit_description_streaming(
    client: &Client<OpenAIConfig>,
    request: CreateResponse,
) -> Result<String> {
    let mut stream = client
        .responses()
        .create_stream(request)
        .await
        .map_err(|e| anyhow!("OpenAI streaming request failed: {e}"))?;

    let mut out = String::new();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| anyhow!("OpenAI stream read failed: {e}"))?;
        match event {
            ResponseStreamEvent::ResponseOutputTextDelta(delta) => out.push_str(&delta.delta),
            ResponseStreamEvent::ResponseOutputTextDone(done) => {
                if out.trim().is_empty() {
                    out.push_str(&done.text);
                }
            }
            ResponseStreamEvent::ResponseRefusalDelta(delta) => out.push_str(&delta.delta),
            ResponseStreamEvent::ResponseRefusalDone(done) => {
                if out.trim().is_empty() {
                    out.push_str(&done.refusal);
                }
            }
            ResponseStreamEvent::ResponseError(err) => {
                return Err(anyhow!(
                    "OpenAI stream error: {} ({})",
                    err.message,
                    err.code.unwrap_or_else(|| "unknown".to_string())
                ));
            }
            _ => {}
        }
    }

    let text = out.trim().to_string();
    if text.is_empty() {
        return Err(anyhow!("OpenAI stream returned no text output"));
    }
    Ok(text)
}

fn map_reasoning_effort(value: ReasoningEffortArg) -> ReasoningEffort {
    match value {
        ReasoningEffortArg::Minimal => ReasoningEffort::Minimal,
        ReasoningEffortArg::Low => ReasoningEffort::Low,
        ReasoningEffortArg::Medium => ReasoningEffort::Medium,
        ReasoningEffortArg::High => ReasoningEffort::High,
    }
}

fn resolve_api_key(settings: &CommitDescSettings) -> Result<String> {
    if settings.requires_openai_auth {
        if let Some(value) = resolve_codex_auth_token(settings) {
            return Ok(value);
        }
        return Ok(DEFAULT_CODEX_AUTH_PLACEHOLDER.to_string());
    }

    if let Some(value) = settings
        .api_key
        .clone()
        .and_then(|v| normalize_string_setting(Some(v)))
    {
        return Ok(value);
    }
    if let Ok(value) = std::env::var(&settings.api_key_env) {
        if let Some(value) = normalize_string_setting(Some(value)) {
            return Ok(value);
        }
    }
    if settings.api_key_env != DEFAULT_API_KEY_ENV {
        if let Ok(value) = std::env::var(DEFAULT_API_KEY_ENV) {
            if let Some(value) = normalize_string_setting(Some(value)) {
                return Ok(value);
            }
        }
    }

    Err(anyhow!(
        "api key mode is enabled, but API key is missing; set --api-key, {}, or {}. \
To use codex-lb auth instead, set requires_openai_auth=true",
        settings.api_key_env,
        DEFAULT_API_KEY_ENV
    ))
}

fn resolve_codex_auth_token(settings: &CommitDescSettings) -> Option<String> {
    if let Ok(value) = std::env::var(&settings.codex_auth_token_env) {
        if let Some(value) = normalize_string_setting(Some(value)) {
            return Some(value);
        }
    }

    for env_name in ["OPENAI_ACCESS_TOKEN", "CODEX_ACCESS_TOKEN"] {
        if env_name == settings.codex_auth_token_env {
            continue;
        }
        if let Ok(value) = std::env::var(env_name) {
            if let Some(value) = normalize_string_setting(Some(value)) {
                return Some(value);
            }
        }
    }

    let auth_path = settings
        .codex_auth_json
        .clone()
        .or_else(default_codex_auth_json_path)?;
    if !auth_path.exists() {
        return None;
    }

    let raw = fs::read_to_string(&auth_path).ok()?;
    let parsed = serde_json::from_str::<CodexAuthFile>(&raw).ok()?;
    if let Some(value) = normalize_string_setting(parsed.openai_api_key) {
        return Some(value);
    }
    let tokens = parsed.tokens?;
    if let Some(value) = normalize_string_setting(tokens.access_token) {
        return Some(value);
    }
    normalize_string_setting(tokens.id_token)
}

fn default_codex_auth_json_path() -> Option<PathBuf> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        if let Some(base) = normalize_string_setting(Some(codex_home)) {
            return Some(PathBuf::from(base).join("auth.json"));
        }
    }
    home_dir().map(|home| home.join(".codex").join("auth.json"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .and_then(|v| normalize_string_setting(Some(v)))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .and_then(|v| normalize_string_setting(Some(v)))
                .map(PathBuf::from)
        })
}

fn normalize_path_setting(path: PathBuf) -> Option<PathBuf> {
    let raw = path.to_string_lossy().trim().to_string();
    if raw.is_empty() {
        return None;
    }
    if raw == "~" {
        return home_dir();
    }
    if let Some(suffix) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return home_dir().map(|home| home.join(suffix));
    }
    Some(PathBuf::from(raw))
}

fn normalize_string_setting(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
