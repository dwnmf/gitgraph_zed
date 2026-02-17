use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::actions::ActionCatalog;
use crate::error::{GitLgError, Result};
use crate::models::GraphQuery;

const DEFAULT_STATE_FILENAME: &str = "state.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppState {
    pub schema_version: u32,
    pub selected_repo_path: Option<PathBuf>,
    pub preferred_git_binary: Option<String>,
    pub default_remote_name: String,
    pub graph_query: GraphQuery,
    pub selected_commit_hashes: Vec<String>,
    pub actions: ActionCatalog,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            selected_repo_path: None,
            preferred_git_binary: None,
            default_remote_name: "origin".to_string(),
            graph_query: GraphQuery::default(),
            selected_commit_hashes: Vec::new(),
            actions: ActionCatalog::with_defaults(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_location() -> Result<PathBuf> {
        let project_dirs = ProjectDirs::from("dev", "GitGraph", "gitgraph")
            .ok_or_else(|| GitLgError::State("cannot resolve project directories".to_string()))?;
        Ok(project_dirs.config_dir().join(DEFAULT_STATE_FILENAME))
    }

    pub fn default_store() -> Result<Self> {
        Ok(Self {
            path: Self::default_location()?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<AppState> {
        if !self.path.exists() {
            return Ok(AppState::default());
        }
        let text = fs::read_to_string(&self.path)
            .map_err(|source| GitLgError::io("reading state file", source))?;
        let mut state: AppState = serde_json::from_str(&text)
            .map_err(|e| GitLgError::State(format!("invalid state json: {}", e)))?;
        if state.schema_version == 0 {
            state.schema_version = CURRENT_SCHEMA_VERSION;
        }
        Ok(state)
    }

    pub fn save(&self, state: &AppState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| GitLgError::io("creating state directory", source))?;
        }
        let text = serde_json::to_string_pretty(state)
            .map_err(|e| GitLgError::State(format!("serialize state failed: {}", e)))?;
        fs::write(&self.path, text).map_err(|source| GitLgError::io("writing state file", source))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{AppState, StateStore};

    #[test]
    fn roundtrip_state_file() {
        let tmp = TempDir::new().expect("tempdir");
        let store = StateStore::at(tmp.path().join("state.json"));

        let mut state = AppState::default();
        state.selected_repo_path = Some(tmp.path().to_path_buf());
        state.default_remote_name = "upstream".to_string();

        store.save(&state).expect("save state");
        let loaded = store.load().expect("load state");
        assert_eq!(loaded.default_remote_name, "upstream");
        assert_eq!(loaded.selected_repo_path, state.selected_repo_path);
    }
}
