//! プロバイダ毎のレート制御（DESIGN §11.3）．
//!
//! 「プロバイダ毎の token-bucket（RPM/TPM 設定可）+ 有界並行（semaphore）」のうち，
//! **token-bucket 部分**をここに置く（semaphore は [`crate::runner`]）．外部 crate を増やさず
//! 小さな内製バケットで実装する．
//!
//! - [`TokenBucket`] — 連続補充式の古典的トークンバケット．時刻を引数で受ける
//!   （`*_at(now)`）ため，実時計に依存せず決定論的に単体テストできる．
//! - [`RateLimiter`] — RPM バケット（1 リクエスト = 1 トークン）と TPM バケット
//!   （消費見積トークン数）を束ね，`acquire` で両者が満たされるまで `tokio` で待つ．

use std::time::{Duration, Instant};

/// RPM / TPM 設定（DESIGN §11.3）．いずれも `None` なら無制限．
#[derive(Debug, Clone, Copy, Default)]
pub struct RateLimit {
    /// requests per minute
    pub rpm: Option<u32>,
    /// tokens per minute
    pub tpm: Option<u32>,
}

impl RateLimit {
    /// 制限なし（テストや Ollama ローカルの既定）．
    pub fn unlimited() -> Self {
        Self::default()
    }

    pub fn new(rpm: Option<u32>, tpm: Option<u32>) -> Self {
        Self { rpm, tpm }
    }
}

/// 連続補充式トークンバケット（DESIGN §11.3）．
///
/// `capacity` 個を上限に，毎秒 `refill_per_sec` 個ずつ補充する．時刻は `*_at(now)` で外部注入し，
/// 実時計への依存を排して決定論的にテストできるようにする．
#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    /// 満杯の状態で作る（`now` 起点）．
    pub fn with_clock(capacity: f64, refill_per_sec: f64, now: Instant) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_per_sec,
            last: now,
        }
    }

    /// `Instant::now()` 起点で満杯に作る．
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self::with_clock(capacity, refill_per_sec, Instant::now())
    }

    /// 現在の残トークン（テスト・観測用）．
    pub fn available(&self) -> f64 {
        self.tokens
    }

    /// `now` までの経過に応じてトークンを補充する（上限は `capacity`）．
    fn refill(&mut self, now: Instant) {
        if now <= self.last {
            return;
        }
        let dt = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + dt * self.refill_per_sec).min(self.capacity);
        self.last = now;
    }

    /// 要求トークンを `capacity` に丸める（1 回で capacity 超を求めると永久に待つのを防ぐ）．
    fn clamp(&self, n: f64) -> f64 {
        n.min(self.capacity).max(0.0)
    }

    /// `now` 時点で `n` トークンを取得できれば消費して `true`，足りなければ `false`．
    pub fn try_acquire_at(&mut self, n: f64, now: Instant) -> bool {
        self.refill(now);
        let need = self.clamp(n);
        if self.tokens + f64::EPSILON >= need {
            self.tokens -= need;
            true
        } else {
            false
        }
    }

    /// `now` 時点で `n` トークンが貯まるまでの待ち時間（足りていれば `Duration::ZERO`）．
    pub fn time_until_at(&mut self, n: f64, now: Instant) -> Duration {
        self.refill(now);
        let need = self.clamp(n);
        if self.tokens + f64::EPSILON >= need {
            return Duration::ZERO;
        }
        if self.refill_per_sec <= 0.0 {
            // 補充されない設定で不足 → 事実上無限だが，暴走を避け大きめの有限値を返す
            return Duration::from_secs(3600);
        }
        let secs = (need - self.tokens) / self.refill_per_sec;
        Duration::from_secs_f64(secs)
    }
}

/// RPM/TPM の 2 バケットを束ねた非同期レート制御（DESIGN §11.3）．
///
/// `acquire` は「1 リクエスト分（RPM）」と「見積トークン分（TPM）」の両方が満たされるまで
/// `tokio::time::sleep` で待つ．std `Mutex` を `await` を跨いで保持しないよう，ロック内では
/// 待ち時間の計算・消費のみ行い，スリープはロック解放後に行う．
pub struct RateLimiter {
    buckets: std::sync::Mutex<Buckets>,
}

struct Buckets {
    rpm: Option<TokenBucket>,
    tpm: Option<TokenBucket>,
}

impl RateLimiter {
    /// 設定からレート制御を作る（`None` の軸は制限しない）．
    pub fn new(limit: RateLimit) -> Self {
        let now = Instant::now();
        let rpm = limit
            .rpm
            .map(|r| TokenBucket::with_clock(r as f64, r as f64 / 60.0, now));
        let tpm = limit
            .tpm
            .map(|t| TokenBucket::with_clock(t as f64, t as f64 / 60.0, now));
        Self {
            buckets: std::sync::Mutex::new(Buckets { rpm, tpm }),
        }
    }

    /// 制限なしのレート制御．
    pub fn unlimited() -> Self {
        Self::new(RateLimit::unlimited())
    }

    /// 1 リクエスト分（RPM）と `est_tokens`（TPM）を確保できるまで待って消費する．
    ///
    /// 両バケットとも `None` なら即座に返る（テストでの実スリープを避ける）．
    pub async fn acquire(&self, est_tokens: u32) {
        loop {
            let wait = {
                let mut b = self.buckets.lock().expect("rate limiter mutex poisoned");
                let now = Instant::now();
                let mut w = Duration::ZERO;
                if let Some(rpm) = b.rpm.as_mut() {
                    w = w.max(rpm.time_until_at(1.0, now));
                }
                if let Some(tpm) = b.tpm.as_mut() {
                    w = w.max(tpm.time_until_at(est_tokens as f64, now));
                }
                if w.is_zero() {
                    // 両軸とも満たせる → 同一時刻で消費して確定
                    if let Some(rpm) = b.rpm.as_mut() {
                        rpm.try_acquire_at(1.0, now);
                    }
                    if let Some(tpm) = b.tpm.as_mut() {
                        tpm.try_acquire_at(est_tokens as f64, now);
                    }
                    return;
                }
                w
            };
            // ロックを解放してから待つ（await を跨いでロックを保持しない）
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_starts_full_and_drains() {
        let t0 = Instant::now();
        let mut b = TokenBucket::with_clock(3.0, 1.0, t0);
        assert!(b.try_acquire_at(1.0, t0));
        assert!(b.try_acquire_at(1.0, t0));
        assert!(b.try_acquire_at(1.0, t0));
        // 使い切ったら失敗
        assert!(!b.try_acquire_at(1.0, t0));
    }

    #[test]
    fn bucket_refills_over_time() {
        let t0 = Instant::now();
        // 容量 2・毎秒 1 補充
        let mut b = TokenBucket::with_clock(2.0, 1.0, t0);
        assert!(b.try_acquire_at(2.0, t0));
        assert!(!b.try_acquire_at(1.0, t0)); // 空
                                             // 1 秒後に 1 トークン補充される
        let t1 = t0 + Duration::from_secs(1);
        assert!(b.try_acquire_at(1.0, t1));
        // さらに空
        assert!(!b.try_acquire_at(1.0, t1));
    }

    #[test]
    fn bucket_refill_is_capped_at_capacity() {
        let t0 = Instant::now();
        let mut b = TokenBucket::with_clock(2.0, 1.0, t0);
        assert!(b.try_acquire_at(2.0, t0)); // 空
                                            // 100 秒経っても容量 2 を超えて貯まらない
        let t = t0 + Duration::from_secs(100);
        assert!(b.try_acquire_at(2.0, t));
        assert!(!b.try_acquire_at(1.0, t));
    }

    #[test]
    fn time_until_matches_refill_rate() {
        let t0 = Instant::now();
        let mut b = TokenBucket::with_clock(10.0, 2.0, t0); // 毎秒 2 補充
        assert_eq!(b.time_until_at(5.0, t0), Duration::ZERO); // 満杯なら 0
        assert!(b.try_acquire_at(10.0, t0)); // 空に
                                             // 6 トークン必要 → 6/2 = 3 秒
        let d = b.time_until_at(6.0, t0);
        assert!((d.as_secs_f64() - 3.0).abs() < 1e-6, "got {d:?}");
    }

    #[test]
    fn request_over_capacity_is_clamped() {
        let t0 = Instant::now();
        let mut b = TokenBucket::with_clock(5.0, 1.0, t0);
        // 容量超の要求は capacity に丸められ，満杯なら取得できる（永久待ちを避ける）
        assert!(b.try_acquire_at(100.0, t0));
        assert_eq!(b.time_until_at(100.0, t0 + Duration::from_secs(5)), {
            // 5 秒で 5 補充 → 満杯 → 0
            Duration::ZERO
        });
    }

    #[tokio::test]
    async fn unlimited_acquire_is_immediate() {
        let rl = RateLimiter::unlimited();
        // 制限なしなら実スリープ無しで多数取得できる
        for _ in 0..1000 {
            rl.acquire(10_000).await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn limited_acquire_blocks_until_refill() {
        // 60 RPM = 毎秒 1 補充・容量 60．60 件即時に取ったあと 61 件目は待つ．
        let rl = RateLimiter::new(RateLimit::new(Some(60), None));
        for _ in 0..60 {
            rl.acquire(0).await; // ほぼ即時（tokio 仮想時計）
        }
        let start = tokio::time::Instant::now();
        rl.acquire(0).await; // 補充待ちが入る
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(500),
            "expected to block for refill, elapsed {elapsed:?}"
        );
    }
}
