//! Case 単位のブートストラップ（DESIGN §11.3）．
//!
//! 多試行 ASR 曲線・union coverage・judge 間差分など，単純な二項割合でない統計は
//! **Case を単位に再標本化**して分布を得る（§11.3）．`rand_chacha::ChaCha8Rng` を seed 固定で
//! 使うため，同一 seed なら区間は完全に一致する（決定論）．
//!
//! 手法は percentile を必須とし，BCa（bias-corrected and accelerated）を推奨オプションとして
//! 持つ．BCa は偏り補正 `z0`（= 中央値の偏りを標準正規で補正）と加速 `a`（= jackknife の
//! 3 次モーメントで歪度を補正）で分位点を調整する．退化して `z0`/`a` が有限に定まらない場合は
//! 自動的に percentile へフォールバックする（例：全成功で分布が 1 点に潰れるケース）．

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::interval::{normal_cdf, normal_quantile, ConfidenceInterval};

/// ブートストラップの分位点法．
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapMethod {
    /// パーセンタイル法（必須・頑健）．
    Percentile,
    /// BCa（推奨）．`z0`/`a` が有限でない退化ケースは Percentile にフォールバックする．
    Bca,
}

/// ソート済み列の分位点（線形補間，NumPy 既定と同じ type-7）．
fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let q = q.clamp(0.0, 1.0);
    let pos = q * (sorted.len() as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = pos - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

/// Case 単位のブートストラップ信頼区間（DESIGN §11.3）．
///
/// `units` は Case 単位のデータ（1 要素 = 1 Case）．`statistic` は再標本化された Case の
/// 参照スライスから統計量を計算する．`n_boot` 回のリサンプルを seed 固定で回す．
/// `point` には全 Case での観測値（`theta_hat`）を入れる．
///
/// 空入力は `NaN` 三つ組を返す．
pub fn bootstrap_ci<T>(
    units: &[T],
    statistic: impl Fn(&[&T]) -> f64,
    n_boot: usize,
    alpha: f64,
    seed: u64,
    method: BootstrapMethod,
) -> ConfidenceInterval {
    let n = units.len();
    if n == 0 || n_boot == 0 {
        return ConfidenceInterval {
            point: f64::NAN,
            lower: f64::NAN,
            upper: f64::NAN,
        };
    }

    let all: Vec<&T> = units.iter().collect();
    let theta_hat = statistic(&all);

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut replicates = Vec::with_capacity(n_boot);
    let mut resample: Vec<&T> = Vec::with_capacity(n);
    for _ in 0..n_boot {
        resample.clear();
        for _ in 0..n {
            let idx = rng.gen_range(0..n);
            resample.push(&units[idx]);
        }
        replicates.push(statistic(&resample));
    }
    replicates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let (q_lo, q_hi) = match method {
        BootstrapMethod::Percentile => (alpha / 2.0, 1.0 - alpha / 2.0),
        BootstrapMethod::Bca => bca_quantiles(&replicates, units, &statistic, theta_hat, alpha)
            .unwrap_or((alpha / 2.0, 1.0 - alpha / 2.0)),
    };

    ConfidenceInterval {
        point: theta_hat,
        lower: quantile_sorted(&replicates, q_lo),
        upper: quantile_sorted(&replicates, q_hi),
    }
}

/// BCa の補正済み分位点 `(alpha1, alpha2)` を返す．退化時は `None`（呼び出し側が percentile に落とす）．
fn bca_quantiles<T>(
    replicates_sorted: &[f64],
    units: &[T],
    statistic: &impl Fn(&[&T]) -> f64,
    theta_hat: f64,
    alpha: f64,
) -> Option<(f64, f64)> {
    let b = replicates_sorted.len();
    if b == 0 {
        return None;
    }
    // 偏り補正 z0: theta_hat 未満の複製割合を標準正規で写す．0/1 に潰れると定義できない．
    let n_less = replicates_sorted.iter().filter(|&&t| t < theta_hat).count();
    let prop = n_less as f64 / b as f64;
    if prop <= 0.0 || prop >= 1.0 {
        return None;
    }
    let z0 = normal_quantile(prop);
    if !z0.is_finite() {
        return None;
    }

    // 加速 a: jackknife（1 件抜き）の 3 次/2 次モーメント比．
    let n = units.len();
    if n < 2 {
        return None;
    }
    let mut jack = Vec::with_capacity(n);
    let mut loo: Vec<&T> = Vec::with_capacity(n - 1);
    for skip in 0..n {
        loo.clear();
        for (j, u) in units.iter().enumerate() {
            if j != skip {
                loo.push(u);
            }
        }
        jack.push(statistic(&loo));
    }
    let mean = jack.iter().sum::<f64>() / n as f64;
    let mut num = 0.0;
    let mut den = 0.0;
    for v in &jack {
        let d = mean - v;
        num += d * d * d;
        den += d * d;
    }
    if den == 0.0 {
        return None;
    }
    let a = num / (6.0 * den.powf(1.5));
    if !a.is_finite() {
        return None;
    }

    let z_lo = normal_quantile(alpha / 2.0);
    let z_hi = normal_quantile(1.0 - alpha / 2.0);
    let adjust = |z: f64| -> f64 {
        let s = z0 + z;
        normal_cdf(z0 + s / (1.0 - a * s))
    };
    let q_lo = adjust(z_lo);
    let q_hi = adjust(z_hi);
    if !q_lo.is_finite() || !q_hi.is_finite() {
        return None;
    }
    Some((q_lo, q_hi))
}
