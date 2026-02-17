use std::time::Instant;

use gitgraph_core::log_parser::{FIELD_SEP, RECORD_SEP, build_graph_rows, parse_git_log_records};
use gitgraph_core::{CommitSearchQuery, filter_commits};

const DEFAULT_10K_THRESHOLD_MS: u128 = 4_000;
const DEFAULT_50K_THRESHOLD_MS: u128 = 20_000;

#[test]
#[ignore = "performance suite for large commit histories"]
fn perf_pipeline_10k_with_threshold() {
    run_perf_case(
        10_000,
        threshold_ms("GITLG_PERF_THRESHOLD_10K_MS", DEFAULT_10K_THRESHOLD_MS),
    );
}

#[test]
#[ignore = "performance suite for large commit histories"]
fn perf_pipeline_50k_with_threshold() {
    run_perf_case(
        50_000,
        threshold_ms("GITLG_PERF_THRESHOLD_50K_MS", DEFAULT_50K_THRESHOLD_MS),
    );
}

fn run_perf_case(commits: usize, threshold_ms: u128) {
    let fixture = synth_log_fixture(commits);

    let start = Instant::now();
    let raw = parse_git_log_records(&fixture).expect("parse fixture");
    let rows = build_graph_rows(raw);
    let filtered = filter_commits(
        &rows,
        &CommitSearchQuery {
            text: "needle".to_string(),
            ..CommitSearchQuery::default()
        },
    )
    .expect("filter commits");
    let elapsed = start.elapsed().as_millis();

    assert_eq!(rows.len(), commits, "parsed+built rows mismatch");
    assert!(!filtered.is_empty(), "needle should match subset");
    assert!(
        elapsed <= threshold_ms,
        "pipeline exceeded threshold: commits={}, elapsed={}ms, threshold={}ms",
        commits,
        elapsed,
        threshold_ms
    );
}

fn threshold_ms(env_key: &str, default_ms: u128) -> u128 {
    std::env::var(env_key)
        .ok()
        .and_then(|v| v.parse::<u128>().ok())
        .unwrap_or(default_ms)
}

fn synth_log_fixture(commits: usize) -> String {
    let mut out = String::with_capacity(commits * 220);
    for index in 0..commits {
        let rev = commits - index;
        let hash = format!("{rev:040x}");
        let short_hash = hash[..7].to_string();
        let parent = if rev > 1 {
            format!("{:040x}", rev - 1)
        } else {
            String::new()
        };
        let subject = if rev % 17 == 0 {
            "optimize parser needle"
        } else {
            "regular commit"
        };
        let body = if rev % 29 == 0 {
            "touches pipeline needle"
        } else {
            "body text"
        };
        let refs = if rev == commits {
            "HEAD -> refs/heads/main"
        } else {
            ""
        };
        out.push_str(&hash);
        out.push(FIELD_SEP);
        out.push_str(&short_hash);
        out.push(FIELD_SEP);
        out.push_str(&parent);
        out.push(FIELD_SEP);
        out.push_str("Perf Bot");
        out.push(FIELD_SEP);
        out.push_str("perf@example.com");
        out.push(FIELD_SEP);
        out.push_str("1700000000");
        out.push(FIELD_SEP);
        out.push_str("1700000000");
        out.push(FIELD_SEP);
        out.push_str(refs);
        out.push(FIELD_SEP);
        out.push_str(subject);
        out.push(FIELD_SEP);
        out.push_str(body);
        out.push(RECORD_SEP);
    }
    out
}
