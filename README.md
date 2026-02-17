# GitTree Zed (Rust)

GitTree Zed is a Rust workspace with Git history tooling for CLI, TUI, and Zed editor integration.

It includes:
- `gitlg-core`: reusable domain/service layer for git graph, search, blame, actions, and state.
- `gitlg-cli`: terminal commands and interactive TUI (`gitgraph`).
- `gitlg-zed`: Zed extension (Wasm) with slash commands.

<img width="3417" height="1339" alt="image" src="https://github.com/user-attachments/assets/8b4c403c-b544-42c9-aaf1-8405a7c19f8c" />


## Features

- Fast commit graph loading (`graph`, `tui`)
- Interactive terminal UI with commit list, file list, patch preview, and search
- Commit description generation for unstaged/staged changes via OpenAI Responses API
- TUI popup flow: generate commit message -> auto-commit -> auto-push
- Actions catalog (preview/run git actions)
- Line blame (`blame`)
- Persisted app state (`state show/set-repo/set-git-binary`)
- Zed slash commands (`/gitlg-log`, `/gitlg-search`, `/gitlg-actions`, etc.)

## Workspace Layout

- `crates/gitlg-core`
- `crates/gitlg-cli`
- `crates/gitlg-zed`

## Requirements

- Rust toolchain (tested with `1.93.1`)
- Git available on `PATH`
- For Zed extension build: `wasm32-wasip2` target

## Windows GNU Toolchain (Recommended in this repo)

Install and use GNU toolchain:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
```

Or run commands explicitly with GNU toolchain prefix:

```powershell
cargo +stable-x86_64-pc-windows-gnu check -p gitlg-cli
cargo +stable-x86_64-pc-windows-gnu test --workspace
```

## Build and Test

Run full test suite:

```powershell
cargo test --workspace
```

Check CLI only:

```powershell
cargo check -p gitlg-cli
```

Build Zed extension artifact:

```powershell
rustup target add wasm32-wasip2
cargo build -p gitlg-zed --target wasm32-wasip2
```

## Install CLI

Install `gitgraph` globally from local source:

```powershell
cargo install --path crates/gitlg-cli --bin gitgraph
```

After install:

```powershell
gitgraph --help
```

## Quick Start

If you are already inside a git repository:

```powershell
gitgraph
```

If not inside a repo, pass `--repo`:

```powershell
cargo run -p gitlg-cli -- tui --repo D:\REALPROJECTS\GitLG
```

## Command Reference

Top-level commands:
- `graph`
- `tui`
- `search`
- `blame`
- `commit-desc`
- `actions`
- `state`
- `validate-repo`

### `graph`

Show git graph as JSON.

Options:
- `--repo <REPO>`
- `--limit <LIMIT>`
- `--skip <SKIP>`
- `--all`
- `--no-stash`
- `--arg <ARG>` (repeatable, forwarded to git log)
- `--pretty`

Example:

```powershell
cargo run -p gitlg-cli -- graph --repo D:\REALPROJECTS\GitLG --limit 200 --all --pretty
```

### `tui`

Start interactive terminal UI.

Options:
- `--repo <REPO>`
- `--limit <LIMIT>`
- `--skip <SKIP>`
- `--all`
- `--no-stash`
- `--arg <ARG>`
- `--graph-style <unicode|ascii>` (default: `ascii`)
- `--max-patch-lines <N>` (default: `0` = unlimited)

Example:

```powershell
cargo run -p gitlg-cli -- tui --repo D:\REALPROJECTS\GitLG --graph-style ascii --max-patch-lines 2500 --limit 900
```

### `search`

Search commit history by text; optionally within a file history.

Options:
- `--repo <REPO>`
- `--text <TEXT>` (required)
- `--file <FILE>`
- `--limit <LIMIT>`
- `--skip <SKIP>`
- `--regex`
- `--case-sensitive`
- `--pretty`

Examples:

```powershell
cargo run -p gitlg-cli -- search --repo D:\REALPROJECTS\GitLG --text checkout --limit 300 --pretty
cargo run -p gitlg-cli -- search --repo D:\REALPROJECTS\GitLG --file src/main.rs --text "run loop" --limit 500 --pretty
```

### `blame`

Show blame info for one file line.

Options:
- `--repo <REPO>`
- `--file <FILE>` (required)
- `--line <LINE>` (required)

Example:

```powershell
cargo run -p gitlg-cli -- blame --repo D:\REALPROJECTS\GitLG --file README.md --line 1
```

### `commit-desc`

Generate commit message text from current uncommitted changes.

Reads:
- `git status --short --untracked-files=all`
- `git diff --staged`
- `git diff`

Options:
- `--repo <REPO>`
- `--model <MODEL>`
- `--reasoning-effort <minimal|low|medium|high>`
- `--base-url <BASE_URL>`
- `--api-key <API_KEY>`
- `--chatgpt-base-url <CHATGPT_BASE_URL>`
- `--requires-openai-auth <true|false>`
- `--codex-auth-json <CODEX_AUTH_JSON>`
- `--codex-auth-token-env <CODEX_AUTH_TOKEN_ENV>`
- `--wire-api <WIRE_API>`
- `--max-output-tokens <MAX_OUTPUT_TOKENS>`
- `--max-diff-chars <MAX_DIFF_CHARS>`

Example (OpenAI API key mode):

```powershell
$env:OPENAI_API_KEY="sk-..."
cargo run -p gitlg-cli -- commit-desc --repo D:\REALPROJECTS\GitLG --model gpt-5-mini --reasoning-effort medium --base-url https://api.openai.com/v1 --wire-api responses
```

### `actions`

Manage action templates.

Subcommands:
- `actions list`
- `actions preview --id <ID> [--param KEY=VALUE] [--option <OPT>] [--ctx KEY=VALUE] [--context-json <FILE>]`
- `actions run --id <ID> [--param KEY=VALUE] [--option <OPT>] [--ctx KEY=VALUE] [--context-json <FILE>]`

Examples:

```powershell
cargo run -p gitlg-cli -- actions list
cargo run -p gitlg-cli -- actions preview --id checkout --param BRANCH_NAME=main
cargo run -p gitlg-cli -- actions run --repo D:\REALPROJECTS\GitLG --id checkout --param BRANCH_NAME=master
cargo run -p gitlg-cli -- actions preview --id merge --ctx SOURCE_BRANCH_NAME=feature --ctx TARGET_BRANCH_NAME=main
```

### `state`

Manage persisted app state.

Subcommands:
- `state show`
- `state set-repo <PATH>`
- `state set-git-binary <BINARY>`

Examples:

```powershell
cargo run -p gitlg-cli -- state show
cargo run -p gitlg-cli -- state set-repo D:\REALPROJECTS\GitLG
cargo run -p gitlg-cli -- state set-git-binary git
```

### `validate-repo`

Quick repo validation by loading a minimal graph.

```powershell
cargo run -p gitlg-cli -- validate-repo --repo D:\REALPROJECTS\GitLG
```

## TUI Controls

Global:
- `q` or `Ctrl+C`: quit
- `Tab` / `Shift+Tab` / `Left` / `Right`: switch pane
- `j` / `k` or arrows: move in active pane
- `g` / `G`: top/bottom in active pane
- `PgUp` / `PgDn`: scroll diff
- `r`: refresh graph
- `/`: focus search input
- `Esc` in normal mode: clear search filter
- Mouse: wheel scroll, left click to select/focus

Search box:
- Type text to filter commits (debounced)
- `Enter`: apply and exit search mode
- `Esc`: cancel search mode

Commit description popup (`m`):
- `m`: generate description popup
- `c`: auto-commit (when generated text is shown)
- `p`: auto-push (after auto-commit success)
- `Esc` / `q`: close popup
- `j` / `k` / `PgUp` / `PgDn` / `g` / `G`: popup scroll

Footer hint in UI:

`status | q quit | tab switch pane | j/k move | g/G top/bottom | PgUp/PgDn diff | r refresh | m commit-desc | mouse: wheel/click`

## Commit Description Configuration (`.config.toml`)

On first `commit-desc` run (or TUI startup), tool auto-creates:

`<repo>/.config.toml`

Default template:

```toml
[gitgraph.openai]
model = "gpt-5-mini"
reasoning_effort = "medium"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
wire_api = "responses"
max_output_tokens = 1200
max_diff_chars = 120000

# codex-lb compatibility (optional):
# When true, codex auth mode is used and API key mode is ignored.
requires_openai_auth = false
codex_auth_json = "~/.codex/auth.json"
codex_auth_token_env = "OPENAI_ACCESS_TOKEN"
# chatgpt_base_url = "http://127.0.0.1:2455"
```

Priority order:
1. CLI flags
2. `<repo>/.config.toml`
3. Built-in defaults

## Auth Modes (Mutually Exclusive)

`requires_openai_auth` controls auth strategy.

### Mode A: API Key mode

Set:

```toml
requires_openai_auth = false
```

Sources used:
1. `--api-key` / `api_key`
2. `api_key_env`
3. `OPENAI_API_KEY` fallback

If none found, command fails with explicit error.

### Mode B: Codex auth mode (`codex-lb`-style)

Set:

```toml
requires_openai_auth = true
```

Sources used:
1. `codex_auth_token_env`
2. `OPENAI_ACCESS_TOKEN`
3. `CODEX_ACCESS_TOKEN`
4. `codex_auth_json`
5. `$CODEX_HOME/auth.json`
6. `~/.codex/auth.json`
7. fallback placeholder token `codex-auth`

Optional request headers for compatibility:
- `wire-api` (default `responses`)
- `chatgpt-base-url` (if configured)
- `requires-openai-auth: true`

### Example: codex-lb

```toml
[gitgraph.openai]
base_url = "http://127.0.0.1:2455/backend-api/codex"
wire_api = "responses"
requires_openai_auth = true
chatgpt_base_url = "http://127.0.0.1:2455"
codex_auth_json = "~/.codex/auth.json"
codex_auth_token_env = "OPENAI_ACCESS_TOKEN"
model = "gpt-5-mini"
reasoning_effort = "medium"
```

## Auto-Commit and Auto-Push Flow

In TUI:
1. Press `m` to generate commit description.
2. Review popup text.
3. Press `c` to run auto-commit (`git add -A` + `git commit -m ...`).
4. Press `p` to run auto-push (`git push`).

If an operation fails, popup shows full error chain.

## State Storage

Persistent app state is stored via `directories::ProjectDirs("dev", "GitLG", "gitlg")` in `state.json`.

State includes:
- selected repo path
- preferred git binary
- default remote name
- graph query defaults
- selected commits
- action catalog

Use `state show` to inspect current values.

## Zed Extension

Extension manifest:
- `crates/gitlg-zed/extension.toml`

Zed tasks:
- `.zed/tasks.json`
- Use `task: spawn` and select `GitLG TUI` or `GitLG TUI (release)`

Implemented slash commands:
- `/gitlg-log [limit]`
- `/gitlg-search [limit=200] [path=src/file.rs] query`
- `/gitlg-actions`
- `/gitlg-action <id> KEY=VALUE +opt:<option-id>`
- `/gitlg-blame <path> <line>`
- `/gitlg-tips`

## Performance Tests

```powershell
cargo test -p gitlg-core perf_pipeline_ -- --ignored --nocapture
```

## Troubleshooting

### "current directory is not a git repository"

Run from a repository root, or pass `--repo <PATH>`.

### `commit-desc` says API key/token is missing

- API key mode: set `OPENAI_API_KEY` (or configure `api_key_env`/`api_key`).
- Codex mode: set `requires_openai_auth=true` and configure `codex_auth_token_env` or `codex_auth_json`.

### TUI popup shows `commit desc error`

Error popup shows full causal chain line by line. Use it to find the failing layer:
- config parsing
- git status/diff read
- OpenAI/codex-lb request
- stream/read failure

### "stream must be set to true"

Client already retries with streaming mode automatically for providers requiring stream-only responses.

### Working tree appears clean unexpectedly

`commit-desc` intentionally ignores changes to `.config.toml` alone. Make sure there are real code/content changes.

## License

MIT
