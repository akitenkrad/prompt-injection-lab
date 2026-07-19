//! §3.4 の非独立性を**実データ**から再現する統合テスト（M9 DoD）．
//!
//! `pil-bench-*` ローダで submodule 実ファイルを直読し（固定 SHA・ネットワーク非依存），
//! `ContentKey` によるベンチ横断重複を数える．§3.4 の完全一致重複は
//! JBB∩AdvBench=11 / JBB∩HarmBench=9 / AdvBench∩HarmBench=0．
//!
//! §3.4 の数値は**完全一致**で測られ，`ContentKey` は正規化キー（§3.5）である．よって
//! ここでは `ContentKey` が数える重複を実測し，その値をそのままアサートする（下の定数は
//! 実測に基づく）．正規化と完全一致で件数が乖離した場合は，本テストが実測値を保持する．

use std::path::PathBuf;

use pil_report::detect_duplicates;

/// リポジトリルート（`third_party/` を含む）．pil-report/../.. で解決する．
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn reproduces_design_3_4_non_independence() {
    let root = repo_root();

    // §3.3 / §3.4 が測定した集合をそのまま使う：
    //   JBB harmful 100 / AdvBench 520（原典 llm-attacks）/ HarmBench text_all 400．
    let jbb = pil_bench_jbb::load_harmful(&root).expect("load JBB harmful");
    let advbench = pil_bench_advbench::load(&root).expect("load AdvBench");
    let harmbench = pil_bench_harmbench::load_text_all(&root).expect("load HarmBench text_all");

    assert_eq!(jbb.len(), 100, "JBB harmful は 100 件");
    assert_eq!(advbench.len(), 520, "AdvBench は 520 件");
    assert_eq!(harmbench.len(), 400, "HarmBench text_all は 400 件");

    let rep = detect_duplicates(&[
        ("JBB", &jbb),
        ("AdvBench", &advbench),
        ("HarmBench", &harmbench),
    ]);

    let jbb_adv = rep.overlap("JBB", "AdvBench").unwrap();
    let jbb_hb = rep.overlap("JBB", "HarmBench").unwrap();
    let adv_hb = rep.overlap("AdvBench", "HarmBench").unwrap();

    // 実測値を可視化（cargo test -- --nocapture）．
    eprintln!("--- §3.4 ContentKey 重複（実測） ---\n{}", rep.render());
    eprintln!("JBB∩AdvBench = {jbb_adv} (設計 §3.4: 11)");
    eprintln!("JBB∩HarmBench = {jbb_hb} (設計 §3.4: 9)");
    eprintln!("AdvBench∩HarmBench = {adv_hb} (設計 §3.4: 0)");

    // §3.4 の期待値（ContentKey 実測がこれに一致することを確認）．
    assert_eq!(jbb_adv, 11, "JBB∩AdvBench");
    assert_eq!(jbb_hb, 9, "JBB∩HarmBench");
    assert_eq!(adv_hb, 0, "AdvBench∩HarmBench");
}
