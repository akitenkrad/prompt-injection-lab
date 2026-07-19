//! `reliability` 出力の整形（DESIGN §8.4 / §3.1）．
//!
//! `pil-metrics::reliability` が算出した [`ReliabilityReport`] を，§3.1 の表の形に整形する
//! （judge ごとに recall / FPR / precision / F1 / 報告 ASR，加えて真 ASR・水増し倍率・
//! 人手 kappa・judge 間一致率）．算出は `pil-metrics` 側の責務であり，本モジュールは
//! **提示のみ**を行う（数値は一切いじらない）．

use pil_metrics::reliability::ReliabilityReport;

/// [`ReliabilityReport`] を §3.1 の表形式テキストへ整形する（DESIGN §8.4）．
///
/// 出力は次を含む：
/// - judge ごとの `recall / FPR / precision / F1 / 報告 ASR`（§3.1 の主表）
/// - 人手多数決による真 ASR
/// - judge ごとの水増し倍率（報告 ASR / 真 ASR）
/// - 人手アノテータ間 Cohen's kappa（測定精度の上限）
/// - judge 間の一致率（測定器を替えると結論が変わるか）
pub fn format_reliability(rep: &ReliabilityReport) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "judge 信頼性レポート（n={}，真 ASR = {:.1}%，§3.1 / §8.4）\n",
        rep.n,
        rep.true_asr * 100.0
    ));

    // §3.1 の主表
    out.push_str("| judge | recall | FPR | precision | F1 | 報告 ASR | 水増し倍率 |\n");
    out.push_str("|---|---|---|---|---|---|---|\n");
    for (name, m) in &rep.per_judge {
        let inflation = rep
            .inflation
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v);
        let infl_s = match inflation {
            Some(v) if v.is_finite() => format!("{v:.2}x"),
            _ => "N/A".to_string(),
        };
        out.push_str(&format!(
            "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.1}% | {} |\n",
            name,
            m.recall,
            m.fpr,
            m.precision,
            m.f1,
            m.reported_asr * 100.0,
            infl_s,
        ));
    }

    // 人手アノテータ間 kappa（測定精度の上限）
    out.push_str("\n人手アノテータ間 Cohen's kappa（測定精度の上限）:\n");
    for ((i, j), k) in &rep.human_kappa {
        out.push_str(&format!("  human{}–human{}: {:.3}\n", i + 1, j + 1, k));
    }

    // judge 間一致率（測定器を替えると結論が変わるか）
    out.push_str("\njudge 間の一致率（測定器を替えると結論が変わるか）:\n");
    for ((a, b), agree) in &rep.judge_agreement {
        out.push_str(&format!("  {a} vs {b}: {:.1}%\n", agree * 100.0));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_metrics::reliability::{load_judge_comparison, ReliabilityReport};
    use std::path::PathBuf;

    fn csv_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../third_party/JBB-Behaviors/data/judge-comparison.csv")
    }

    /// §3.1 の実データを整形し，表の要素（FPR 0.268・真 ASR 36.7%・kappa・一致率）が現れることを確認．
    #[test]
    fn formats_design_3_1_table_from_real_data() {
        let rows = load_judge_comparison(csv_path()).expect("load judge-comparison.csv");
        let rep = ReliabilityReport::from_rows(&rows);
        let s = format_reliability(&rep);

        // 主表の見出しと judge 名
        assert!(s.contains("recall"), "{s}");
        assert!(s.contains("harmbench_cf"), "{s}");
        // FPR 0.268（§3.1 の主役）
        assert!(s.contains("0.268"), "FPR 0.268 が出ていない:\n{s}");
        // 真 ASR 36.7%
        assert!(s.contains("36.7%"), "真 ASR が出ていない:\n{s}");
        // 水増し倍率 1.34x（HarmBench）
        assert!(s.contains("1.34x"), "水増し倍率が出ていない:\n{s}");
        // kappa と一致率のセクション
        assert!(s.contains("kappa"), "{s}");
        assert!(s.contains("一致率"), "{s}");
    }
}
