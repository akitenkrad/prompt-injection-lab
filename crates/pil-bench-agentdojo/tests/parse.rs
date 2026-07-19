//! network-free な結果 JSON / 列挙 JSON パースの回帰（DESIGN §4.1 / §5.3 / §10）．
//!
//! agentdojo の pip インストールや Python 実行を要さず，永続化された `TaskResults` 形状の fixture と
//! 列挙器出力の fixture だけで Verdict 写像・provenance・決定論を緑にする．

use std::path::PathBuf;

use pil_bench_agentdojo::{
    load_enumeration, parse_result, AgentDojoCase, UndecidableReason, Verdict, BENCHMARK_VERSION,
    COMMIT, UPSTREAM,
};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn parses_security_true_as_success() {
    let r = parse_result(&fixture("agentdojo_result_security_true.json")).expect("parse");
    assert!(r.security);
    assert!(r.utility);
    assert_eq!(
        r.verdict(),
        Verdict::Success,
        "security=true は注入成功（§10）"
    );
}

#[test]
fn parses_security_false_as_failure() {
    let r = parse_result(&fixture("agentdojo_result_security_false.json")).expect("parse");
    assert!(!r.security);
    assert!(r.utility);
    assert_eq!(r.verdict(), Verdict::Failure);
}

#[test]
fn parses_error_as_undecidable() {
    let r = parse_result(&fixture("agentdojo_result_error.json")).expect("parse");
    assert!(r.error.is_some());
    match r.verdict() {
        Verdict::Undecidable {
            reason: UndecidableReason::ProviderError { message },
        } => assert!(message.contains("context window")),
        other => panic!("expected ProviderError Undecidable, got {other:?}"),
    }
}

#[test]
fn measurement_preserves_utility_from_fixture() {
    // security=false でも utility=true を潰さず保持する（§5.3）．
    let r = parse_result(&fixture("agentdojo_result_security_false.json")).expect("parse");
    let m = r.to_measurement();
    assert_eq!(m.verdict, Verdict::Failure);
    assert!(m.raw.contains("\"utility\":true"));
    assert!(m.score.is_none());
}

#[test]
fn enumeration_maps_to_emulated_cases() {
    let cases = load_enumeration(&fixture("enumeration.json")).expect("load enumeration");
    assert_eq!(cases.len(), 5);
    for c in &cases {
        assert_eq!(c.env_kind, pil_bench_agentdojo::EnvKind::Emulated);
        assert_eq!(c.source.upstream, UPSTREAM);
        assert_eq!(c.source.commit, COMMIT);
        assert!(!c.benign);
        assert_eq!(
            c.labels.get("benchmark_version").map(String::as_str),
            Some(BENCHMARK_VERSION)
        );
    }
    // 決定論: 列挙経由の Case と直接構築の Case は同一 identity．
    let direct = AgentDojoCase::new("banking", "user_task_0", "injection_task_0").to_case();
    assert_eq!(cases[0].id, direct.id);
}

#[test]
fn enumeration_cases_are_distinct() {
    let cases = load_enumeration(&fixture("enumeration.json")).expect("load enumeration");
    let mut ids: Vec<_> = cases.iter().map(|c| c.id.full().to_string()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 5, "5 件は全て別 identity");
}
