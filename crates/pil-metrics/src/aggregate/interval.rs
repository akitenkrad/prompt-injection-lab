//! 二項割合の信頼区間（DESIGN §11.3）．
//!
//! 単発 ASR は二項割合であり，既定は **Wilson score 区間**とする（極端な p・小 n でも被覆が
//! 良い）．Clopper–Pearson は保守的すぎるため，最悪ケース報告用のオプションに留める（§11.3）．
//! いずれも正規/ベータ分位点を `statrs` から取り，結果は必ず `[0,1]` にクランプする．

use serde::{Deserialize, Serialize};
use statrs::distribution::{Beta, ContinuousCDF, Normal};

/// 95% 区間の既定 z（標準正規の 0.975 分位点）．`Normal::inverse_cdf(0.975)` に一致する定数．
pub const Z_95: f64 = 1.959_963_985;

/// 点推定と下限・上限（すべて `[0,1]` にクランプ済み）．
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceInterval {
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
}

/// 標準正規の逆 CDF（Wilson の z・bootstrap BCa の z0/加速で使う）．
pub fn normal_quantile(p: f64) -> f64 {
    Normal::new(0.0, 1.0)
        .expect("standard normal is well-defined")
        .inverse_cdf(p)
}

/// 標準正規の CDF（bootstrap BCa の分位点補正で使う）．
pub fn normal_cdf(x: f64) -> f64 {
    Normal::new(0.0, 1.0)
        .expect("standard normal is well-defined")
        .cdf(x)
}

/// Wilson score 区間（DESIGN §11.3 の既定）．
///
/// `successes` 件 / `n` 試行，`z` は正規分位点（95% は [`Z_95`]）．`n = 0` は
/// `point = NaN`, `[0,1]` を返す．p = 0/1 の端も中心が内側に寄るため区間は退化しない．
pub fn wilson_interval(successes: usize, n: usize, z: f64) -> ConfidenceInterval {
    if n == 0 {
        return ConfidenceInterval {
            point: f64::NAN,
            lower: 0.0,
            upper: 1.0,
        };
    }
    let n_f = n as f64;
    let p = successes as f64 / n_f;
    let z2 = z * z;
    let denom = 1.0 + z2 / n_f;
    let center = (p + z2 / (2.0 * n_f)) / denom;
    let margin = (z / denom) * (p * (1.0 - p) / n_f + z2 / (4.0 * n_f * n_f)).sqrt();
    ConfidenceInterval {
        point: p,
        lower: (center - margin).clamp(0.0, 1.0),
        upper: (center + margin).clamp(0.0, 1.0),
    }
}

/// Clopper–Pearson（exact）区間（DESIGN §11.3 のオプション：最悪ケース報告用）．
///
/// ベータ分位点で構成する．`k = 0` は下限 0，`k = n` は上限 1 に固定し，退化を避ける．
/// `alpha` は両側の有意水準（95% なら 0.05）．`n = 0` は `point = NaN`, `[0,1]`．
pub fn clopper_pearson_interval(successes: usize, n: usize, alpha: f64) -> ConfidenceInterval {
    if n == 0 {
        return ConfidenceInterval {
            point: f64::NAN,
            lower: 0.0,
            upper: 1.0,
        };
    }
    let k = successes as f64;
    let n_f = n as f64;
    let p = k / n_f;
    let lower = if successes == 0 {
        0.0
    } else {
        Beta::new(k, n_f - k + 1.0)
            .expect("beta shape params are positive")
            .inverse_cdf(alpha / 2.0)
    };
    let upper = if successes == n {
        1.0
    } else {
        Beta::new(k + 1.0, n_f - k)
            .expect("beta shape params are positive")
            .inverse_cdf(1.0 - alpha / 2.0)
    };
    ConfidenceInterval {
        point: p,
        lower: lower.clamp(0.0, 1.0),
        upper: upper.clamp(0.0, 1.0),
    }
}
