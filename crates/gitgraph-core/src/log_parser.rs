use crate::error::{GitLgError, Result};
use crate::models::{GitRef, GitRefKind, GraphEdge, GraphRow};

pub const FIELD_SEP: char = '\u{001f}';
pub const RECORD_SEP: char = '\u{001e}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCommit {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_unix: i64,
    pub committed_unix: i64,
    pub refs: Vec<GitRef>,
    pub subject: String,
    pub body: String,
}

pub fn parse_git_log_records(stdout: &str) -> Result<Vec<RawCommit>> {
    let mut commits = Vec::new();
    for raw_record in stdout.split(RECORD_SEP) {
        let record = raw_record.trim_matches(['\r', '\n', ' ']);
        if record.is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.splitn(10, FIELD_SEP).collect();
        if fields.len() != 10 {
            return Err(GitLgError::Parse(format!(
                "expected 10 fields, got {} in record {:?}",
                fields.len(),
                record
            )));
        }
        let authored_unix = fields[5].parse::<i64>().map_err(|e| {
            GitLgError::Parse(format!(
                "invalid authored unix timestamp {:?}: {}",
                fields[5], e
            ))
        })?;
        let committed_unix = fields[6].parse::<i64>().map_err(|e| {
            GitLgError::Parse(format!(
                "invalid committed unix timestamp {:?}: {}",
                fields[6], e
            ))
        })?;
        let parents = if fields[2].trim().is_empty() {
            Vec::new()
        } else {
            fields[2]
                .split_whitespace()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        };

        commits.push(RawCommit {
            hash: fields[0].to_string(),
            short_hash: fields[1].to_string(),
            parents,
            author_name: fields[3].to_string(),
            author_email: fields[4].to_string(),
            authored_unix,
            committed_unix,
            refs: parse_refs(fields[7]),
            subject: fields[8].to_string(),
            body: fields[9].to_string(),
        });
    }
    Ok(commits)
}

pub fn build_graph_rows(commits: Vec<RawCommit>) -> Vec<GraphRow> {
    let mut active_lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        let lane = find_or_allocate_lane(&commit.hash, &mut active_lanes);

        // A hash can appear in multiple lanes due to merge ancestry.
        // Collapse duplicates after choosing primary lane.
        for (idx, lane_hash) in active_lanes.iter_mut().enumerate() {
            if idx != lane && lane_hash.as_deref() == Some(commit.hash.as_str()) {
                *lane_hash = None;
            }
        }

        let mut edges = Vec::new();
        if let Some(first_parent) = commit.parents.first() {
            active_lanes[lane] = Some(first_parent.clone());
            edges.push(GraphEdge {
                to_lane: lane,
                parent_hash: first_parent.clone(),
            });
        } else {
            active_lanes[lane] = None;
        }

        for parent in commit.parents.iter().skip(1) {
            let target_lane = find_or_allocate_lane(parent, &mut active_lanes);
            edges.push(GraphEdge {
                to_lane: target_lane,
                parent_hash: parent.clone(),
            });
        }

        while active_lanes.last().is_some_and(Option::is_none) {
            active_lanes.pop();
        }

        rows.push(GraphRow {
            hash: commit.hash,
            short_hash: commit.short_hash,
            parents: commit.parents,
            author_name: commit.author_name,
            author_email: commit.author_email,
            authored_unix: commit.authored_unix,
            committed_unix: commit.committed_unix,
            subject: commit.subject,
            body: commit.body,
            refs: commit.refs,
            lane,
            active_lane_count: active_lanes.len(),
            edges,
        });
    }
    rows
}

fn find_or_allocate_lane(hash: &str, active_lanes: &mut Vec<Option<String>>) -> usize {
    if let Some((idx, _)) = active_lanes
        .iter()
        .enumerate()
        .find(|(_, slot)| slot.as_deref() == Some(hash))
    {
        return idx;
    }
    if let Some((idx, slot)) = active_lanes
        .iter_mut()
        .enumerate()
        .find(|(_, slot)| slot.is_none())
    {
        *slot = Some(hash.to_string());
        return idx;
    }
    active_lanes.push(Some(hash.to_string()));
    active_lanes.len() - 1
}

fn parse_refs(decorations: &str) -> Vec<GitRef> {
    let cleaned = decorations.trim();
    if cleaned.is_empty() {
        return Vec::new();
    }

    cleaned
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(parse_ref_token)
        .collect()
}

fn parse_ref_token(token: &str) -> GitRef {
    if let Some((left, right)) = token.split_once(" -> ") {
        let left_name = simplify_ref_name(left.trim());
        let target = simplify_ref_name(right.trim());
        let kind = if left.trim() == "HEAD" {
            GitRefKind::Head
        } else {
            classify_ref(left.trim())
        };
        return GitRef {
            kind,
            name: left_name,
            target: Some(target),
        };
    }

    if let Some(rest) = token.strip_prefix("tag: ") {
        return GitRef {
            kind: GitRefKind::Tag,
            name: simplify_ref_name(rest.trim()),
            target: None,
        };
    }

    if token == "HEAD" {
        return GitRef {
            kind: GitRefKind::Head,
            name: "HEAD".to_string(),
            target: None,
        };
    }

    GitRef {
        kind: classify_ref(token),
        name: simplify_ref_name(token),
        target: None,
    }
}

fn classify_ref(raw: &str) -> GitRefKind {
    if raw.starts_with("refs/heads/") {
        return GitRefKind::LocalBranch;
    }
    if raw.starts_with("refs/remotes/") {
        return GitRefKind::RemoteBranch;
    }
    if raw.starts_with("refs/tags/") {
        return GitRefKind::Tag;
    }
    if raw == "refs/stash" {
        return GitRefKind::Stash;
    }
    GitRefKind::Other
}

fn simplify_ref_name(raw: &str) -> String {
    raw.strip_prefix("refs/heads/")
        .or_else(|| raw.strip_prefix("refs/remotes/"))
        .or_else(|| raw.strip_prefix("refs/tags/"))
        .unwrap_or(raw)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        FIELD_SEP, GitRefKind, RECORD_SEP, build_graph_rows, parse_git_log_records, parse_ref_token,
    };

    #[test]
    fn parses_one_record() {
        let rec = format!(
            "aaaaaaaa{}aaaaaaa{}bbbbbbbb{}Alice{}alice@example.com{}1700000000{}1700000001{}HEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1.0{}Subject{}Body{}",
            FIELD_SEP,
            FIELD_SEP,
            FIELD_SEP,
            FIELD_SEP,
            FIELD_SEP,
            FIELD_SEP,
            FIELD_SEP,
            FIELD_SEP,
            FIELD_SEP,
            RECORD_SEP
        );
        let parsed = parse_git_log_records(&rec).expect("parse records");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].hash, "aaaaaaaa");
        assert_eq!(parsed[0].parents, vec!["bbbbbbbb"]);
        assert_eq!(parsed[0].refs.len(), 3);
    }

    #[test]
    fn parses_ref_types() {
        let head = parse_ref_token("HEAD -> refs/heads/main");
        assert_eq!(head.kind, GitRefKind::Head);
        assert_eq!(head.target.as_deref(), Some("main"));

        let remote = parse_ref_token("refs/remotes/origin/main");
        assert_eq!(remote.kind, GitRefKind::RemoteBranch);
        assert_eq!(remote.name, "origin/main");

        let tag = parse_ref_token("tag: refs/tags/v1.0.0");
        assert_eq!(tag.kind, GitRefKind::Tag);
        assert_eq!(tag.name, "v1.0.0");
    }

    #[test]
    fn assigns_lanes_for_merge() {
        let rec = format!(
            "c3{f}c3{f}p1 p2{f}A{f}a@e{f}10{f}10{f}{f}merge{f}{r}p1{f}p1{f}p0{f}A{f}a@e{f}9{f}9{f}{f}parent1{f}{r}p2{f}p2{f}p0{f}A{f}a@e{f}8{f}8{f}{f}parent2{f}{r}p0{f}p0{f}{f}A{f}a@e{f}7{f}7{f}{f}root{f}{r}",
            f = FIELD_SEP,
            r = RECORD_SEP
        );
        let raw = parse_git_log_records(&rec).expect("valid records");
        let rows = build_graph_rows(raw);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].hash, "c3");
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[0].edges.len(), 2);
        assert!(rows[0].edges.iter().any(|e| e.to_lane == 1));
    }
}
