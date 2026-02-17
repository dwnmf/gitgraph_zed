pub mod actions;
pub mod error;
pub mod git;
pub mod log_parser;
pub mod models;
pub mod search;
pub mod service;
pub mod state;

pub use actions::{
    ActionCatalog, ActionContext, ActionOption, ActionParam, ActionRequest, ActionScope,
    ActionTemplate, ResolvedAction,
};
pub use error::{GitLgError, Result};
pub use git::{GitOutput, GitRunner};
pub use models::{
    BlameInfo, BranchInfo, CommitSearchQuery, FileChange, GitRef, GitRefKind, GraphData, GraphEdge,
    GraphQuery, GraphRow,
};
pub use search::filter_commits;
pub use service::{ActionExecutionResult, GitLgService};
pub use state::{AppState, StateStore};
