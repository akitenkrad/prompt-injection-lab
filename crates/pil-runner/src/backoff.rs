//! 429 リトライのバックオフ方針（DESIGN §11.3）．
//!
//! 「429 は `Retry-After` を尊重し，無ければ指数バックオフ + ジッタ」を純関数として実装する．
//! ジッタは呼び出し側が注入する RNG（`rand` / `rand_chacha`）で引くため，**seed を固定すれば
//! 決定論的**になり単体テストできる（テスト容易性の要件）．
//!
//! - [`BackoffPolicy::raw_backoff`] — ジッタ無しの素の指数バックオフ（上限つき）．純粋・決定論的．
//! - [`BackoffPolicy::delay`] — `Retry-After` があれば**それを尊重**（そのまま返す）し，無ければ
//!   `raw_backoff` に **equal jitter** を掛ける（結果は必ず `[raw/2, raw]` に収まる）．

use std::time::Duration;

use rand::Rng;

/// 指数バックオフ + ジッタの方針（DESIGN §11.3）．
///
/// `delay = min(max, base * factor^retry)` を基準に，`Retry-After` 尊重または equal jitter を適用する．
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackoffPolicy {
    /// リトライ 0 回目の基準待ち時間
    pub base: Duration,
    /// 指数の底（通常 2.0）
    pub factor: f64,
    /// 待ち時間の上限（暴走防止）
    pub max: Duration,
}

impl Default for BackoffPolicy {
    /// base 200ms・factor 2.0・max 30s（一般的な API リトライの既定）．
    fn default() -> Self {
        Self {
            base: Duration::from_millis(200),
            factor: 2.0,
            max: Duration::from_secs(30),
        }
    }
}

impl BackoffPolicy {
    pub fn new(base: Duration, factor: f64, max: Duration) -> Self {
        Self { base, factor, max }
    }

    /// ジッタ無しの素の指数バックオフ `min(max, base * factor^retry)`（決定論的・純粋）．
    ///
    /// `retry` は 0 始まり（初回リトライが 0）．オーバーフローは `max` で飽和させる．
    pub fn raw_backoff(&self, retry: u32) -> Duration {
        let base_ms = self.base.as_millis() as f64;
        let max_ms = self.max.as_millis() as f64;
        let scaled = base_ms * self.factor.powi(retry as i32);
        // NaN / Inf / 上限超過はすべて max に丸める
        let ms = if scaled.is_finite() {
            scaled.min(max_ms)
        } else {
            max_ms
        };
        Duration::from_millis(ms as u64)
    }

    /// 実際の待ち時間を決める（DESIGN §11.3）．
    ///
    /// - `retry_after` が `Some` なら，サーバ指示を**尊重してそのまま返す**（ジッタを掛けない）．
    /// - `None` なら `raw_backoff(retry)` に **equal jitter** を掛け，`[raw/2, raw]` の一様乱数を返す．
    ///
    /// ジッタは `rng` から引くため，同一 seed の RNG を渡せば決定論的（再現・テスト可能）．
    pub fn delay<R: Rng + ?Sized>(
        &self,
        retry: u32,
        retry_after: Option<Duration>,
        rng: &mut R,
    ) -> Duration {
        if let Some(ra) = retry_after {
            // サーバの Retry-After を尊重（§11.3）．
            return ra;
        }
        let raw = self.raw_backoff(retry);
        let raw_ms = raw.as_millis() as u64;
        let half = raw_ms / 2;
        if half == 0 {
            return raw;
        }
        // equal jitter: [half, half + rand(0..=half)] = [raw/2, raw]
        let jitter = rng.gen_range(0..=half);
        Duration::from_millis(half + jitter)
    }
}

/// エラーメッセージから `Retry-After`（秒）を抽出する（DESIGN §11.3）．
///
/// `LlmError` は `Retry-After` を構造化して持たないため，プロバイダが業務エラーの文面に埋めた
/// `retry-after=<秒>`（大文字小文字・区切り記号を問わない）を拾う．見つからなければ `None`．
///
/// 例: `"429 Too Many Requests; Retry-After: 3"` → `Some(3s)`．
pub fn parse_retry_after(message: &str) -> Option<Duration> {
    let lower = message.to_ascii_lowercase();
    let idx = lower.find("retry-after")?;
    let rest = &lower[idx + "retry-after".len()..];
    // 先頭の非数字（`=`・`:`・空白）を読み飛ばし，続く数字列を取る
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn policy() -> BackoffPolicy {
        BackoffPolicy::new(Duration::from_millis(100), 2.0, Duration::from_secs(10))
    }

    #[test]
    fn raw_backoff_is_exponential() {
        let p = policy();
        assert_eq!(p.raw_backoff(0), Duration::from_millis(100));
        assert_eq!(p.raw_backoff(1), Duration::from_millis(200));
        assert_eq!(p.raw_backoff(2), Duration::from_millis(400));
        assert_eq!(p.raw_backoff(3), Duration::from_millis(800));
    }

    #[test]
    fn raw_backoff_is_capped_at_max() {
        let p = policy();
        // 100ms * 2^10 = 102_400ms > 10s の上限 → max に飽和
        assert_eq!(p.raw_backoff(10), Duration::from_secs(10));
        // 巨大な retry でもオーバーフローせず max
        assert_eq!(p.raw_backoff(1000), Duration::from_secs(10));
    }

    #[test]
    fn delay_respects_retry_after() {
        let p = policy();
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let ra = Duration::from_secs(5);
        // Retry-After があれば retry 回数やジッタに関係なくそのまま
        assert_eq!(p.delay(0, Some(ra), &mut rng), ra);
        assert_eq!(p.delay(7, Some(ra), &mut rng), ra);
    }

    #[test]
    fn delay_jitter_within_bounds() {
        let p = policy();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for retry in 0..6 {
            let raw = p.raw_backoff(retry);
            for _ in 0..200 {
                let d = p.delay(retry, None, &mut rng);
                assert!(
                    d >= raw / 2 && d <= raw,
                    "delay {d:?} out of [{:?}, {:?}] at retry {retry}",
                    raw / 2,
                    raw
                );
            }
        }
    }

    #[test]
    fn delay_is_deterministic_for_same_seed() {
        let p = policy();
        let mut r1 = ChaCha8Rng::seed_from_u64(7);
        let mut r2 = ChaCha8Rng::seed_from_u64(7);
        for retry in 0..5 {
            assert_eq!(p.delay(retry, None, &mut r1), p.delay(retry, None, &mut r2));
        }
    }

    #[test]
    fn parse_retry_after_variants() {
        assert_eq!(
            parse_retry_after("429 Too Many Requests; Retry-After: 3"),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            parse_retry_after("provider returned an error: retry-after=12"),
            Some(Duration::from_secs(12))
        );
        assert_eq!(
            parse_retry_after("RETRY-AFTER 0"),
            Some(Duration::from_secs(0))
        );
        assert_eq!(parse_retry_after("some other 500 error"), None);
        assert_eq!(parse_retry_after("retry-after: soon"), None);
    }
}
