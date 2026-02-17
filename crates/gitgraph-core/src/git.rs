use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{GitLgError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct GitRunner {
    git_binary: String,
    env: BTreeMap<String, String>,
}

impl Default for GitRunner {
    fn default() -> Self {
        Self::new("git")
    }
}

impl GitRunner {
    pub fn new(git_binary: impl Into<String>) -> Self {
        Self {
            git_binary: git_binary.into(),
            env: BTreeMap::new(),
        }
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn git_binary(&self) -> &str {
        &self.git_binary
    }

    pub fn validate_repo(&self, repo_path: &Path) -> Result<()> {
        if !repo_path.exists() || !repo_path.is_dir() {
            return Err(GitLgError::InvalidRepository(repo_path.to_path_buf()));
        }
        let out = self.exec(
            repo_path,
            &["rev-parse".to_string(), "--is-inside-work-tree".to_string()],
            true,
        )?;
        if out.stdout.trim() == "true" {
            return Ok(());
        }
        Err(GitLgError::InvalidRepository(repo_path.to_path_buf()))
    }

    pub fn exec(
        &self,
        repo_path: &Path,
        args: &[String],
        allow_non_zero: bool,
    ) -> Result<GitOutput> {
        let mut cmd = Command::new(&self.git_binary);
        cmd.current_dir(repo_path)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        let output = cmd
            .output()
            .map_err(|source| GitLgError::io("running git command", source))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let result = GitOutput {
            stdout,
            stderr,
            exit_code: output.status.code(),
        };
        if output.status.success() || allow_non_zero {
            return Ok(result);
        }
        Err(GitLgError::GitCommandFailed {
            program: self.git_binary.clone(),
            args: args.to_vec(),
            exit_code: result.exit_code,
            stderr: result.stderr,
            stdout: result.stdout,
        })
    }

    pub fn exec_shell(
        &self,
        repo_path: &Path,
        script: &str,
        allow_non_zero: bool,
    ) -> Result<GitOutput> {
        #[cfg(target_os = "windows")]
        let (program, args): (&str, Vec<String>) =
            ("cmd", vec!["/C".to_string(), script.to_string()]);
        #[cfg(not(target_os = "windows"))]
        let (program, args): (&str, Vec<String>) =
            ("sh", vec!["-lc".to_string(), script.to_string()]);

        let mut cmd = Command::new(program);
        cmd.current_dir(repo_path)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        let output = cmd
            .output()
            .map_err(|source| GitLgError::io("running shell git script", source))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let result = GitOutput {
            stdout,
            stderr,
            exit_code: output.status.code(),
        };
        if output.status.success() || allow_non_zero {
            return Ok(result);
        }
        Err(GitLgError::GitCommandFailed {
            program: program.to_string(),
            args,
            exit_code: result.exit_code,
            stderr: result.stderr,
            stdout: result.stdout,
        })
    }

    pub fn discover_repo_root(&self, start_path: &Path) -> Result<PathBuf> {
        let out = self.exec(
            start_path,
            &["rev-parse".to_string(), "--show-toplevel".to_string()],
            false,
        )?;
        let root = out.stdout.trim();
        if root.is_empty() {
            return Err(GitLgError::InvalidRepository(start_path.to_path_buf()));
        }
        Ok(PathBuf::from(root))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::GitRunner;

    fn has_git() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn init_repo(tmp: &Path) {
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp)
            .output()
            .expect("git init must run");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(tmp)
            .output()
            .expect("set user.name");
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(tmp)
            .output()
            .expect("set user.email");
        fs::write(tmp.join("README.md"), "hello\n").expect("write readme");
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(tmp)
            .output()
            .expect("add");
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(tmp)
            .output()
            .expect("commit");
    }

    #[test]
    fn validates_git_repository() {
        if !has_git() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        init_repo(tmp.path());

        let runner = GitRunner::default();
        runner
            .validate_repo(tmp.path())
            .expect("repo should be valid");
    }
}
