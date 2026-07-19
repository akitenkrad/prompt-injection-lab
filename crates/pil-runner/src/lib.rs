//! `pil-runner` — 多試行・有界並行・レート制御・中断再開の実行器（DESIGN §11.3 / IMPLEMENTATION_PLAN M7）．
//!
//! Case × Attack × attempt を回し，**1 回の生成に複数の測定器をぶら下げて**
//! `Trial.measurements: Vec<Measurement>` を作る（§5.5）．注入された `pil_llm::LlmProvider` と
//! `pil_metrics::instrument::Instrument` 群で動くため，既定ビルドは `MockProvider` により
//! ネットワーク非依存でテストできる（§6.1）．
//!
//! # 構成
//!
//! - [`backoff`] — 429 リトライのバックオフ方針（`Retry-After` 尊重・指数 + ジッタ，§11.3）．純関数．
//! - [`rate_limit`] — プロバイダ毎 token-bucket（RPM/TPM）と非同期レート制御（§11.3）．
//! - [`checkpoint`] — `(CaseId, instrument, attempt, seed)` 単位の append-only JSONL による
//!   中断再開（§6.2 / §11.3）．完了タプルガード + 1 生成 = 1 バッチ追記で冪等．
//! - [`runner`] — 上記を束ねた実行器 [`Runner`]．
//!
//! # 設計上の要点
//!
//! - **seed = f(attempt)**（§11.4）: 送信 seed は `pil_llm::seed_for_attempt(base_seed, attempt)`．
//!   再現性（同 `(base, attempt)` で同一生成）と独立サンプル（attempt 毎に別 seed）を両立する．
//! - **中断再開の冪等性**（§11.3）: 完了生成の全タプルが揃っていればスキップし，プロバイダを
//!   一切呼ばない．よって再開時に既完了タプルの生成回数は 0．
//! - **1 生成に複数測定器**（§5.5）: 生成は 1 回，そこに全測定器を当てて `Vec<Measurement>` を得る．

pub mod backoff;
pub mod checkpoint;
pub mod error;
pub mod rate_limit;
pub mod runner;

pub use backoff::{parse_retry_after, BackoffPolicy};
pub use checkpoint::{Checkpoint, CheckpointRecord, TupleKey};
pub use error::{JobError, JobErrorKind, RunnerError};
pub use rate_limit::{RateLimit, RateLimiter, TokenBucket};
pub use runner::{BoxedInstrument, RunConfig, RunOutcome, Runner};
