//! ASR の提示（信頼区間 + undecidable 件数の常時併記，DESIGN §8.1）．
//!
//! §8.1 の「ASR は 12.3% ± 3.1」を全件から出す，という提示側の責務を担う．一次資料の主張の核心
//! （信頼区間を出さない・判定不能を黙って潰す）への回答として，本モジュールの提示は
//! **常に信頼区間と undecidable 件数を同時に携える**．CI 無しや undecidable 件数無しの
//! ASR 文字列を作る経路は用意しない．
//!
//! [`AsrPresentation`] は機械可読（serde）表現であり，[`AsrPresentation::render`] が人間可読
//! 文字列を返す．入力は `aggregate` の [`AsrResult`]（CI・undecidable を既に持つ）か，本 crate の
//! [`crate::BinarizedCounts`]（二値化で潰した件数を持つ）のいずれからでも構築できる．

use serde::{Deserialize, Serialize};

use pil_metrics::aggregate::{wilson_interval, AsrResult, Z_95};

use crate::binarize::BinarizedCounts;

/// ASR の提示単位（DESIGN §8.1）．
///
/// **CI と undecidable 件数を常に含む**．どのフィールドも欠かせない構造にすることで，
/// 「点推定だけ」「区間だけ」の提示を型で防ぐ．
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AsrPresentation {
    /// 点推定（`successes / (successes + failures)`）．判定可能が 0 件なら `NaN`．
    pub asr: f64,
    /// 信頼区間の下限（既定は Wilson score，§11.3）．
    pub ci_lower: f64,
    /// 信頼区間の上限．
    pub ci_upper: f64,
    /// 信頼水準（%）．既定は 95．
    pub confidence_pct: f64,
    /// 分子（success）件数．
    pub successes: usize,
    /// 判定可能な分母（`successes + failures`）．
    pub decided: usize,
    /// 分母から外した `Undecidable` 件数．常に併記する（§8.1 / §5.3）．
    pub undecidable: usize,
}

impl AsrPresentation {
    /// `aggregate` の [`AsrResult`] から構築する（CI・undecidable はそのまま転記）．
    ///
    /// `confidence_pct` は表示上の信頼水準（`AsrResult` の CI と整合させて渡す；既定 95）．
    pub fn from_asr_result(res: &AsrResult, confidence_pct: f64) -> Self {
        Self {
            asr: res.asr,
            ci_lower: res.ci.lower,
            ci_upper: res.ci.upper,
            confidence_pct,
            successes: res.successes,
            decided: res.successes + res.failures,
            undecidable: res.undecidable,
        }
    }

    /// 二値化結果 [`BinarizedCounts`] から構築する（Wilson 95% 区間を自前で算出）．
    ///
    /// `undecidable` は二値化で潰した件数（[`BinarizedCounts::undecidable_collapsed`]）を転記する —
    /// 潰しても件数は消えない（§5.3）．
    pub fn from_binarized(counts: &BinarizedCounts) -> Self {
        let decided = counts.denominator();
        let ci = wilson_interval(counts.successes, decided, Z_95);
        Self {
            asr: counts.asr(),
            ci_lower: ci.lower,
            ci_upper: ci.upper,
            confidence_pct: 95.0,
            successes: counts.successes,
            decided,
            undecidable: counts.undecidable_collapsed,
        }
    }

    /// 人間可読の 1 行提示（DESIGN §8.1）．
    ///
    /// 例: `ASR = 12.3% [8.5%, 17.2%] (95% CI; n=130 判定可能, undecidable=4)`．
    /// CI と undecidable 件数を必ず含む．判定可能が 0 件のときは `ASR = N/A` とし，undecidable のみ示す．
    pub fn render(&self) -> String {
        if self.asr.is_nan() {
            return format!(
                "ASR = N/A (判定可能 0 件; undecidable={})",
                self.undecidable
            );
        }
        format!(
            "ASR = {:.1}% [{:.1}%, {:.1}%] ({:.0}% CI; n={} 判定可能, undecidable={})",
            self.asr * 100.0,
            self.ci_lower * 100.0,
            self.ci_upper * 100.0,
            self.confidence_pct,
            self.decided,
            self.undecidable,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_metrics::aggregate::wilson_interval;

    use crate::binarize::{binarize, BinarizationPolicy};
    use pil_core::{UndecidableReason, Verdict};

    fn asr_result(successes: usize, failures: usize, undecidable: usize) -> AsrResult {
        let denom = successes + failures;
        AsrResult {
            successes,
            failures,
            undecidable,
            asr: if denom == 0 {
                f64::NAN
            } else {
                successes as f64 / denom as f64
            },
            ci: wilson_interval(successes, denom, Z_95),
        }
    }

    /// 提示は常に CI と undecidable 件数を含む（§8.1 の DoD）．
    #[test]
    fn presentation_always_carries_ci_and_undecidable() {
        let res = asr_result(50, 50, 7);
        let p = AsrPresentation::from_asr_result(&res, 95.0);
        assert_eq!(p.undecidable, 7);
        assert!(p.ci_lower < p.asr && p.asr < p.ci_upper);
        let s = p.render();
        assert!(s.contains("CI"), "{s}");
        assert!(s.contains("undecidable=7"), "{s}");
        assert!(s.contains('['), "区間が出ていない: {s}");
    }

    /// 二値化結果からの提示も undecidable（潰した件数）を必ず携える（§5.3 / §8.1）．
    #[test]
    fn from_binarized_keeps_collapsed_count() {
        let verdicts = vec![
            Verdict::Success,
            Verdict::Failure,
            Verdict::Undecidable {
                reason: UndecidableReason::ResponseTruncated,
            },
        ];
        let counts = binarize(&verdicts, BinarizationPolicy::ExcludeUndecidable);
        let p = AsrPresentation::from_binarized(&counts);
        assert_eq!(p.decided, 2);
        assert_eq!(p.undecidable, 1);
        assert!((p.asr - 0.5).abs() < 1e-12);
        assert!(p.render().contains("undecidable=1"));
    }

    #[test]
    fn nan_asr_renders_na_with_undecidable() {
        let res = asr_result(0, 0, 3);
        let p = AsrPresentation::from_asr_result(&res, 95.0);
        let s = p.render();
        assert!(s.contains("N/A"), "{s}");
        assert!(s.contains("undecidable=3"), "{s}");
    }

    #[test]
    fn serde_roundtrip() {
        let res = asr_result(12, 88, 4);
        let p = AsrPresentation::from_asr_result(&res, 95.0);
        let json = serde_json::to_string(&p).unwrap();
        let back: AsrPresentation = serde_json::from_str(&json).unwrap();
        // 整数フィールドは厳密一致，f64 は末尾 ULP の揺れを許容する
        // （serde_json の f64 直列化/解析は最終桁で往復安定でないことがある）．
        assert_eq!(p.successes, back.successes);
        assert_eq!(p.decided, back.decided);
        assert_eq!(p.undecidable, back.undecidable);
        assert!((p.asr - back.asr).abs() < 1e-12);
        assert!((p.ci_lower - back.ci_lower).abs() < 1e-12);
        assert!((p.ci_upper - back.ci_upper).abs() < 1e-12);
        assert!((p.confidence_pct - back.confidence_pct).abs() < 1e-12);
    }
}
