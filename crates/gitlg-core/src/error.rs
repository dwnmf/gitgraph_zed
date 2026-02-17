use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, GitLgError>;

#[derive(Debug, Error)]
pub enum GitLgError {
    #[error("I/O failure while {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("git command failed: `{program}` {args:?}, exit_code={exit_code:?}, stderr={stderr}")]
    GitCommandFailed {
        program: String,
        args: Vec<String>,
        exit_code: Option<i32>,
        stderr: String,
        stdout: String,
    },

    #[error("invalid git repository: {0}")]
    InvalidRepository(PathBuf),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("missing required placeholder value: {0}")]
    MissingPlaceholder(String),

    #[error("state error: {0}")]
    State(String),
}

impl GitLgError {
    pub fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }
}
