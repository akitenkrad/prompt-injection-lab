//! `pil reliability` — judge 信頼性の開示（DESIGN §3.1 / §8.4 / IMPLEMENTATION_PLAN M2・M10）．
//!
//! JBB の `data/judge-comparison.csv`（300 件）から §3.1 の表（recall / FPR / precision / F1 /
//! 報告 ASR / 水増し倍率 / 人手 kappa / judge 間一致率）を算出して表示する．LLM もネットワークも
//! 一切呼ばない（§8.4）．成果物は `results/reliability_<TS>/` に人間可読 + 機械可読で保存する．

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use pil_metrics::reliability::{load_judge_comparison, ReliabilityReport};
use pil_report::format_reliability;

use crate::commands::{make_results_dir, write_text, Provenance};

/// judge-comparison.csv の submodule 相対パス（§8.4）．
const JUDGE_COMPARISON_REL: &str = "third_party/JBB-Behaviors/data/judge-comparison.csv";

/// `pil reliability` の本体．
///
/// `repo_root` から judge-comparison.csv を直読し，[`ReliabilityReport`] を算出して表示・保存する．
pub fn run(repo_root: &Path) -> Result<()> {
    let csv_path = repo_root.join(JUDGE_COMPARISON_REL);
    let rows = load_judge_comparison(&csv_path)
        .with_context(|| format!("judge-comparison.csv を読めません: {}", csv_path.display()))?;

    let report = ReliabilityReport::from_rows(&rows);
    let table = format_reliability(&report);

    // 標準出力に §3.1 の表を出す．
    println!("{table}");

    // 成果物保存（human-readable + machine-readable）．
    let dir = make_results_dir(repo_root, "reliability")?;
    write_text(&dir, "reliability.txt", &table)?;
    let machine = serde_json::to_string_pretty(&report)?;
    write_text(&dir, "reliability.json", &machine)?;

    let prov = Provenance::new(
        "reliability",
        None,
        json!({
            "input_csv": JUDGE_COMPARISON_REL,
            "n_rows": report.n,
            "true_asr": report.true_asr,
        }),
    );
    prov.write(&dir)?;

    println!("\n成果物: {}", dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// リポジトリルート（`crates/pil-cli/../..`）．
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// reliability 経路が §3.1 の数値（FPR 0.268 / 真 ASR 36.7%）を再現する（M10 DoD）．
    #[test]
    fn reliability_path_reproduces_design_3_1() {
        let root = repo_root();
        let csv_path = root.join(JUDGE_COMPARISON_REL);
        let rows = load_judge_comparison(&csv_path).expect("load judge-comparison.csv");
        let report = ReliabilityReport::from_rows(&rows);

        assert_eq!(report.n, 300);
        assert!(
            (report.true_asr - 0.367).abs() < 1e-3,
            "true_asr={}",
            report.true_asr
        );

        let hb = report.judge("harmbench_cf").expect("harmbench_cf present");
        assert!((hb.fpr - 0.268).abs() < 1e-3, "fpr={}", hb.fpr);
        assert!((hb.recall - 0.873).abs() < 1e-3, "recall={}", hb.recall);

        // 整形テキストに §3.1 の主役（FPR 0.268・真 ASR 36.7%）が現れる．
        let table = format_reliability(&report);
        assert!(table.contains("0.268"), "{table}");
        assert!(table.contains("36.7%"), "{table}");
    }
}
