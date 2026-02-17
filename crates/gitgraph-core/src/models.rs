use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitRefKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Stash,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRef {
    pub kind: GitRefKind,
    pub name: String,
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub full_ref: String,
    pub is_remote: bool,
    pub remote_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub to_lane: usize,
    pub parent_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphRow {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_unix: i64,
    pub committed_unix: i64,
    pub subject: String,
    pub body: String,
    pub refs: Vec<GitRef>,
    pub lane: usize,
    pub active_lane_count: usize,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphData {
    pub repository: PathBuf,
    pub generated_at_unix: i64,
    pub query: GraphQuery,
    pub commits: Vec<GraphRow>,
    pub branches: Vec<BranchInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphQuery {
    pub limit: usize,
    pub skip: usize,
    pub all_refs: bool,
    pub include_stash_ref: bool,
    pub additional_args: Vec<String>,
}

impl Default for GraphQuery {
    fn default() -> Self {
        Self {
            limit: 15_000,
            skip: 0,
            all_refs: true,
            include_stash_ref: true,
            additional_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitSearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub use_regex: bool,
    pub file_path: Option<String>,
    pub include_hash: bool,
    pub include_subject: bool,
    pub include_body: bool,
    pub include_author: bool,
    pub include_email: bool,
    pub include_refs: bool,
}

impl Default for CommitSearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            case_sensitive: false,
            use_regex: false,
            file_path: None,
            include_hash: true,
            include_subject: true,
            include_body: true,
            include_author: true,
            include_email: true,
            include_refs: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlameInfo {
    pub file: PathBuf,
    pub line: usize,
    pub commit_hash: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time_unix: i64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub added: Option<u32>,
    pub removed: Option<u32>,
}
