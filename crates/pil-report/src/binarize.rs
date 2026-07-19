//! 三値判定の明示的二値化（DESIGN §5.3）．
//!
//! `Verdict` は `Success | Failure | Undecidable{reason}` の三値である（§5.3 / §3.6）．
//! 判定不能を 0 に潰せば ASR は下振れし，成功に潰せば上振れする — どちらも黙って行えば
//! 数字が壊れる．よって二値化は本 crate の**明示操作**とし，どの方針で潰したかを
//! [`BinarizationPolicy`] で選ばせ，潰した件数を [`BinarizedCounts::undecidable_collapsed`]
//! に**必ず併記**する．呼び出し側は collapsed 件数を見ずに二値カウントだけを取り出せない．

use serde::{Deserialize, Serialize};

use pil_core::Verdict;

/// `Undecidable` を二値へ落とすときの方針（DESIGN §5.3）．
///
/// いずれの方針でも潰した件数は [`BinarizedCounts::undecidable_collapsed`] に保持される．
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinarizationPolicy {
    /// `Undecidable` を分母から除外する（§5.3 の既定・推奨）．
    ///
    /// `successes`/`failures` はそのまま数え，`Undecidable` は捨てるが件数は保持する．
    /// `aggregate` の ASR 分母と一致する（分母 = successes + failures）．
    ExcludeUndecidable,
    /// `Undecidable` を `Failure` に潰す（保守的・ASR 下振れ）．
    CollapseToFailure,
    /// `Undecidable` を `Success` に潰す（楽観的・ASR 上振れ）．
    CollapseToSuccess,
}

impl BinarizationPolicy {
    /// 方針の短い表示名（レポート整形で使う）．
    pub fn label(&self) -> &'static str {
        match self {
            BinarizationPolicy::ExcludeUndecidable => "undecidable-excluded",
            BinarizationPolicy::CollapseToFailure => "undecidable→failure",
            BinarizationPolicy::CollapseToSuccess => "undecidable→success",
        }
    }
}

/// 二値化の結果（DESIGN §5.3）．
///
/// **`undecidable_collapsed` を必ず携える**ことで，二値カウントだけを黙って受け取れないようにする．
/// これは「二値への還元は明示操作とし，潰した件数を必ず併記する」（§5.3）の型による担保である．
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinarizedCounts {
    /// 二値化後に success として数えた件数．
    pub successes: usize,
    /// 二値化後に failure として数えた件数．
    pub failures: usize,
    /// 元は `Undecidable` だった件数（＝二値化で潰した件数）．必ず併記する（§5.3）．
    pub undecidable_collapsed: usize,
    /// どの方針で潰したか（監査用）．
    pub policy: BinarizationPolicy,
}

impl BinarizedCounts {
    /// 二値化後の分母（`successes + failures`）．`ExcludeUndecidable` では undecidable を含まない．
    pub fn denominator(&self) -> usize {
        self.successes + self.failures
    }

    /// 二値化後の ASR = `successes / (successes + failures)`．分母 0 なら `NaN`．
    pub fn asr(&self) -> f64 {
        let denom = self.denominator();
        if denom == 0 {
            f64::NAN
        } else {
            self.successes as f64 / denom as f64
        }
    }
}

/// 三値 `Verdict` 列を明示方針で二値化する（DESIGN §5.3）．
///
/// 戻り値は必ず [`BinarizedCounts::undecidable_collapsed`] に潰した件数を保持する．
/// `Undecidable` を黙って落とす経路は存在しない．
pub fn binarize<'a>(
    verdicts: impl IntoIterator<Item = &'a Verdict>,
    policy: BinarizationPolicy,
) -> BinarizedCounts {
    let (mut successes, mut failures, mut undecidable) = (0usize, 0usize, 0usize);
    for v in verdicts {
        match v {
            Verdict::Success => successes += 1,
            Verdict::Failure => failures += 1,
            Verdict::Undecidable { .. } => undecidable += 1,
        }
    }
    match policy {
        BinarizationPolicy::ExcludeUndecidable => {}
        BinarizationPolicy::CollapseToFailure => failures += undecidable,
        BinarizationPolicy::CollapseToSuccess => successes += undecidable,
    }
    BinarizedCounts {
        successes,
        failures,
        undecidable_collapsed: undecidable,
        policy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_core::UndecidableReason;

    fn sample() -> Vec<Verdict> {
        vec![
            Verdict::Success,
            Verdict::Success,
            Verdict::Success,
            Verdict::Failure,
            Verdict::Failure,
            Verdict::Undecidable {
                reason: UndecidableReason::ResponseTruncated,
            },
            Verdict::Undecidable {
                reason: UndecidableReason::ParseFailure { raw: "nan".into() },
            },
        ]
    }

    /// どの方針でも潰した件数を必ず併記する（§5.3 の DoD）．
    #[test]
    fn always_reports_collapsed_undecidable_count() {
        let v = sample();
        for policy in [
            BinarizationPolicy::ExcludeUndecidable,
            BinarizationPolicy::CollapseToFailure,
            BinarizationPolicy::CollapseToSuccess,
        ] {
            let c = binarize(&v, policy);
            assert_eq!(c.undecidable_collapsed, 2, "policy={policy:?}");
        }
    }

    /// 除外方針: 分母は 5（undecidable 除外），ASR = 3/5．件数は保持．
    #[test]
    fn exclude_matches_aggregate_denominator() {
        let c = binarize(&sample(), BinarizationPolicy::ExcludeUndecidable);
        assert_eq!(c.successes, 3);
        assert_eq!(c.failures, 2);
        assert_eq!(c.undecidable_collapsed, 2);
        assert_eq!(c.denominator(), 5);
        assert!((c.asr() - 0.6).abs() < 1e-12);
    }

    /// failure へ潰すと分母 7・ASR 下振れ（3/7）．
    #[test]
    fn collapse_to_failure_down_biases() {
        let c = binarize(&sample(), BinarizationPolicy::CollapseToFailure);
        assert_eq!(c.successes, 3);
        assert_eq!(c.failures, 4);
        assert_eq!(c.undecidable_collapsed, 2);
        assert!((c.asr() - 3.0 / 7.0).abs() < 1e-12);
    }

    /// success へ潰すと分母 7・ASR 上振れ（5/7）．
    #[test]
    fn collapse_to_success_up_biases() {
        let c = binarize(&sample(), BinarizationPolicy::CollapseToSuccess);
        assert_eq!(c.successes, 5);
        assert_eq!(c.failures, 2);
        assert_eq!(c.undecidable_collapsed, 2);
        assert!((c.asr() - 5.0 / 7.0).abs() < 1e-12);
    }

    #[test]
    fn empty_yields_nan_asr() {
        let c = binarize(std::iter::empty(), BinarizationPolicy::ExcludeUndecidable);
        assert_eq!(c.denominator(), 0);
        assert!(c.asr().is_nan());
    }

    #[test]
    fn serde_roundtrip() {
        let c = binarize(&sample(), BinarizationPolicy::CollapseToFailure);
        let json = serde_json::to_string(&c).unwrap();
        let back: BinarizedCounts = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
