use regex::RegexBuilder;

use crate::error::{GitLgError, Result};
use crate::models::{CommitSearchQuery, GraphRow};

pub fn filter_commits(rows: &[GraphRow], query: &CommitSearchQuery) -> Result<Vec<GraphRow>> {
    let needle = query.text.trim();
    if needle.is_empty() {
        return Ok(rows.to_vec());
    }

    if query.use_regex {
        let regex = RegexBuilder::new(needle)
            .case_insensitive(!query.case_sensitive)
            .build()
            .map_err(|e| GitLgError::Parse(format!("invalid regex {:?}: {}", needle, e)))?;
        return Ok(rows
            .iter()
            .filter(|row| search_parts(row, query, |part| regex.is_match(part)))
            .cloned()
            .collect());
    }

    if query.case_sensitive {
        return Ok(rows
            .iter()
            .filter(|row| search_parts(row, query, |part| part.contains(needle)))
            .cloned()
            .collect());
    }

    let normalized_needle = needle.to_lowercase();
    Ok(rows
        .iter()
        .filter(|row| {
            search_parts(row, query, |part| part.to_lowercase().contains(&normalized_needle))
        })
        .cloned()
        .collect())
}

fn search_parts(
    row: &GraphRow,
    query: &CommitSearchQuery,
    mut matches: impl FnMut(&str) -> bool,
) -> bool {
    if query.include_hash && (matches(row.hash.as_str()) || matches(row.short_hash.as_str())) {
        return true;
    }
    if query.include_subject && matches(row.subject.as_str()) {
        return true;
    }
    if query.include_body && matches(row.body.as_str()) {
        return true;
    }
    if query.include_author && matches(row.author_name.as_str()) {
        return true;
    }
    if query.include_email && matches(row.author_email.as_str()) {
        return true;
    }
    if query.include_refs {
        for git_ref in &row.refs {
            if matches(git_ref.name.as_str()) {
                return true;
            }
            if let Some(target) = git_ref.target.as_deref()
                && matches(target)
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::models::{CommitSearchQuery, GitRef, GitRefKind, GraphRow};

    use super::filter_commits;

    fn sample_rows() -> Vec<GraphRow> {
        vec![
            GraphRow {
                hash: "aaaaaaaa".to_string(),
                short_hash: "aaaaaaa".to_string(),
                parents: vec!["bbbbbbbb".to_string()],
                author_name: "Alice".to_string(),
                author_email: "alice@example.com".to_string(),
                authored_unix: 10,
                committed_unix: 10,
                subject: "Add parser".to_string(),
                body: "Adds commit parser".to_string(),
                refs: vec![GitRef {
                    kind: GitRefKind::LocalBranch,
                    name: "main".to_string(),
                    target: None,
                }],
                lane: 0,
                active_lane_count: 1,
                edges: vec![],
            },
            GraphRow {
                hash: "cccccccc".to_string(),
                short_hash: "ccccccc".to_string(),
                parents: vec!["bbbbbbbb".to_string()],
                author_name: "Bob".to_string(),
                author_email: "bob@example.com".to_string(),
                authored_unix: 11,
                committed_unix: 11,
                subject: "Fix ui".to_string(),
                body: "Nothing about parser".to_string(),
                refs: vec![],
                lane: 0,
                active_lane_count: 1,
                edges: vec![],
            },
        ]
    }

    #[test]
    fn filters_substring_case_insensitive() {
        let mut q = CommitSearchQuery::default();
        q.text = "PARSER".to_string();
        let filtered = filter_commits(&sample_rows(), &q).expect("search");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filters_regex() {
        let mut q = CommitSearchQuery::default();
        q.use_regex = true;
        q.text = "^Fix\\s".to_string();
        let filtered = filter_commits(&sample_rows(), &q).expect("search");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].author_name, "Bob");
    }
}
