# Feature Mapping: GitLG -> GitTree_Zed

Date: 2026-02-16

## Legend
- `Done`: implemented and verified by tests/build.
- `In Progress`: partial implementation exists, needs parity hardening.
- `Blocked`: constrained by current Zed extension surface.

## Core Git / Graph
- Parse git log into structured commit graph: `Done`
- Lane/edge graph model for merge topology: `Done`
- Branch list extraction with upstream metadata: `Done`
- Stash-ref aware loading: `Done`

## Search / Filtering
- Search by subject/body/author/email/hash/refs: `Done`
- Regex search mode: `Done`
- Search CLI command outputting filtered graph JSON: `Done`
- Search by file contents in historical snapshots: `Done`

## Actions
- Load full default action catalog from GitLG JSON: `Done`
- Scopes (`global`, `commit`, `commits`, `stash`, `tag`, `branch`, `branch-drop`): `Done`
- Placeholder expansion (`{...}`, `$1..$N`): `Done`
- Dynamic placeholders (`{GIT_CONFIG:...}`, `{GIT_EXEC:...}`): `Done`
- Shell/composite actions (`&&`, `||`, `;`): `Done`
- Short action id compatibility resolver (e.g. `checkout`): `Done`

## State / Persistence
- Persistent state file with query + selected repo + actions: `Done`
- Backward-compatible state deserialization after schema expansion: `Done`

## Blame / History Utilities
- Blame line API in core service: `Done`
- CLI blame command: `Done`

## Zed Extension
- Dev extension manifest and wasm build: `Done`
- `/gitlg-log`: `Done`
- `/gitlg-search`: `Done`
- `/gitlg-actions`: `Done`
- `/gitlg-action`: `Done`
- `/gitlg-blame`: `Done`
- `/gitlg-tips`: `Done`
- Native rich graph panel inside Zed sidebar/editor area: `Blocked`
Reason: current Zed extension API surface does not provide VS Code webview-equivalent custom UI embedding for this use case.

## CLI TUI
- Full-screen interactive commit graph list + details pane: `Done`
- Keyboard navigation (`j/k`, arrows, `g/G`, `r`, `/`, `Esc`, `q`): `Done`
- Mouse support in TUI list (wheel + click select): `Done`
- Right pane file list with per-file `+/-` stats and patch viewer: `Done`
- Incremental search apply in TUI: `Done`
- Inline action context flags (`--ctx KEY=VALUE`) for actions preview/run: `Done`

## Testing / Verification
- Unit tests for parser/search/actions/state/service: `Done`
- Workspace test run: `Done`
- Wasm build for `gitlg-zed`: `Done`
- Large-repo performance benchmark suite (10k/50k with thresholds): `Done`
