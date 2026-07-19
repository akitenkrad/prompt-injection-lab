//! 多試行・有界並行・レート制御・中断再開の実行器（DESIGN §5.5 / §6.2 / §11.3 / §11.4）．
//!
//! Case × Attack × attempt を回し，**1 回の生成に複数の測定器をぶら下げて**
//! `Trial.measurements: Vec<Measurement>` を作る（§5.5）．注入された [`LlmProvider`] と
//! [`Instrument`] 群で動くため，既定ビルドは `MockProvider` によりネットワーク非依存にテストできる．
//!
//! # 実行の流れ
//!
//! 1. **多試行ループ**: attempt `1..=attempts`．送信 seed は `seed_for_attempt(base_seed, attempt)`
//!    で導出（再現性と独立サンプルの両立，§11.4）．
//! 2. **プロンプト生成**: `pil_attacks::render(case, attack)` で変換適用後の `rendered_prompt` を
//!    作ってから生成する（キャッシュ・チェックポイントの単位はこの最終プロンプト，§6.2）．
//! 3. **有界並行**: `tokio::sync::Semaphore` で同時実行数を制限する．
//! 4. **レート制御**: プロバイダ毎 token-bucket（RPM/TPM）．429 は `Retry-After` 尊重の指数
//!    バックオフ + ジッタ（[`crate::backoff`] / [`crate::rate_limit`]）．
//! 5. **中断再開**: `(CaseId, instrument, attempt, seed)` 単位の append-only JSONL に完了を刻み，
//!    再起動時は完了生成をスキップする（[`crate::checkpoint`]）．
//! 6. **複数測定器**: 生成した 1 応答に全 [`Instrument`] を当て，`Vec<Measurement>` を作る（§5.5）．

use std::path::PathBuf;
use std::sync::Arc;

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use pil_attacks::render;
use pil_core::{AttackRef, Case, ModelRef, Trial};
use pil_llm::{GenerateRequest, LlmConfig, LlmError, LlmProvider};
use pil_metrics::instrument::Instrument;

use crate::backoff::{parse_retry_after, BackoffPolicy};
use crate::checkpoint::{Checkpoint, CheckpointRecord, TupleKey};
use crate::error::{JobError, JobErrorKind, RunnerError};
use crate::rate_limit::{RateLimit, RateLimiter};

/// 実行器に注入する測定器（1 応答に複数当てる，§5.5）．並行タスク間で共有するため `Send + Sync`．
pub type BoxedInstrument = Box<dyn Instrument + Send + Sync>;

/// 実行設定（DESIGN §11.3 / §11.4）．
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// 呼び出すモデル
    pub model: ModelRef,
    /// 温度・基底 seed・max_tokens・system．送信 seed は attempt から導出（§11.4）
    pub llm_config: LlmConfig,
    /// 多試行回数（1..=attempts）．Anthropic 式 1/10/100 回開示（§6.2）
    pub attempts: u32,
    /// 有界並行の同時実行数（semaphore permit 数，§11.3）
    pub concurrency: usize,
    /// プロバイダ毎レート制御（RPM/TPM，§11.3）
    pub rate: RateLimit,
    /// 429 バックオフ方針（§11.3）
    pub backoff: BackoffPolicy,
    /// logprobs 要求個数（§6.3．None なら要求しない）
    pub top_logprobs: Option<u32>,
    /// 1 生成あたりの最大リトライ回数
    pub max_retries: u32,
    /// チェックポイント JSONL のパス（§11.3）
    pub checkpoint_path: PathBuf,
}

impl RunConfig {
    /// 最小構成（テスト向け）: 単発・並行 1・レート無制限・既定バックオフ・リトライ 3．
    pub fn new(model: ModelRef, checkpoint_path: impl Into<PathBuf>) -> Self {
        Self {
            model,
            llm_config: LlmConfig::default(),
            attempts: 1,
            concurrency: 1,
            rate: RateLimit::unlimited(),
            backoff: BackoffPolicy::default(),
            top_logprobs: None,
            max_retries: 3,
            checkpoint_path: checkpoint_path.into(),
        }
    }

    pub fn attempts(mut self, n: u32) -> Self {
        self.attempts = n.max(1);
        self
    }

    pub fn concurrency(mut self, n: usize) -> Self {
        self.concurrency = n.max(1);
        self
    }

    pub fn rate(mut self, rate: RateLimit) -> Self {
        self.rate = rate;
        self
    }

    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }
}

/// 実行結果（DESIGN §5.5 / §11.3）．
#[derive(Debug, Default)]
pub struct RunOutcome {
    /// 再開分（チェックポイント由来）+ 今回生成分の Trial．
    pub trials: Vec<Trial>,
    /// ジョブ単位の失敗（全体は続行．§11.3）．
    pub errors: Vec<JobError>,
    /// 今回実際に生成したジョブ数（プロバイダを呼んだ数）．
    pub generated: usize,
    /// チェックポイント済みでスキップしたジョブ数．
    pub skipped: usize,
}

/// 実行器本体（DESIGN §11.3）．
pub struct Runner<P: LlmProvider> {
    provider: Arc<P>,
    instruments: Arc<Vec<BoxedInstrument>>,
    config: RunConfig,
}

impl<P: LlmProvider + Send + Sync + 'static> Runner<P> {
    /// プロバイダ・測定器・設定を注入して作る（DESIGN §4.1 の依存注入）．
    pub fn new(provider: Arc<P>, instruments: Vec<BoxedInstrument>, config: RunConfig) -> Self {
        Self {
            provider,
            instruments: Arc::new(instruments),
            config,
        }
    }

    /// この生成（case, attack, attempt）が生む完了タプル群を列挙する（スキップ判定・追記に使う）．
    fn expected_keys(
        &self,
        case: &Case,
        attack: &AttackRef,
        attempt: u32,
        seed: u64,
    ) -> Vec<TupleKey> {
        self.instruments
            .iter()
            .map(|inst| {
                let r = inst.reference();
                TupleKey {
                    case: case.id.clone(),
                    attack: attack.clone(),
                    instrument_name: r.name,
                    instrument_version: r.version,
                    attempt,
                    seed,
                }
            })
            .collect()
    }

    /// Case × Attack × attempt を実行する（DESIGN §11.3）．
    ///
    /// チェックポイントを読み込み，未完了の生成だけをプロバイダに投げ，完了を都度刻む．
    /// 再開分と今回生成分を合わせた `Trial` を返す．
    pub async fn run(
        &self,
        cases: &[Case],
        attacks: &[AttackRef],
    ) -> Result<RunOutcome, RunnerError> {
        let checkpoint = Arc::new(Checkpoint::load(&self.config.checkpoint_path).await?);

        // 再開分の Trial（読み込み時点まで）を先に確保する．
        let resumed = checkpoint.resumed_trials().await;

        let limiter = Arc::new(RateLimiter::new(self.config.rate));
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency));

        let mut join_set: JoinSet<JobResult> = JoinSet::new();
        let mut skipped = 0usize;

        for case in cases {
            for attack in attacks {
                for attempt in 1..=self.config.attempts {
                    let seed = self.config.llm_config.effective_seed(attempt);
                    let keys = self.expected_keys(case, attack, attempt, seed);

                    // 完了タプルが全て揃っていればスキップ（プロバイダを呼ばない = 二重生成しない）．
                    if checkpoint.contains_all(&keys).await {
                        skipped += 1;
                        continue;
                    }

                    let ctx = JobCtx {
                        provider: self.provider.clone(),
                        instruments: self.instruments.clone(),
                        checkpoint: checkpoint.clone(),
                        limiter: limiter.clone(),
                        semaphore: semaphore.clone(),
                        backoff: self.config.backoff,
                        model: self.config.model.clone(),
                        llm_config: self.config.llm_config.clone(),
                        top_logprobs: self.config.top_logprobs,
                        max_retries: self.config.max_retries,
                        case: case.clone(),
                        attack: attack.clone(),
                        attempt,
                    };
                    join_set.spawn(run_one_job(ctx));
                }
            }
        }

        let mut trials = resumed;
        let mut errors = Vec::new();
        let mut generated = 0usize;

        while let Some(joined) = join_set.join_next().await {
            match joined.map_err(|e| RunnerError::Join(e.to_string()))? {
                JobResult::Done(trial) => {
                    generated += 1;
                    trials.push(trial);
                }
                JobResult::Failed(err) => errors.push(err),
            }
        }

        Ok(RunOutcome {
            trials,
            errors,
            generated,
            skipped,
        })
    }
}

/// 1 ジョブに必要な共有状態と入力（並行タスクへ move する）．
struct JobCtx<P: LlmProvider> {
    provider: Arc<P>,
    instruments: Arc<Vec<BoxedInstrument>>,
    checkpoint: Arc<Checkpoint>,
    limiter: Arc<RateLimiter>,
    semaphore: Arc<Semaphore>,
    backoff: BackoffPolicy,
    model: ModelRef,
    llm_config: LlmConfig,
    top_logprobs: Option<u32>,
    max_retries: u32,
    case: Case,
    attack: AttackRef,
    attempt: u32,
}

/// 1 ジョブの結果．
enum JobResult {
    Done(Trial),
    Failed(JobError),
}

/// リトライして良い（一時的な）失敗か（DESIGN §11.3）．
fn is_retryable(err: &LlmError) -> bool {
    matches!(err, LlmError::Provider(_) | LlmError::Network(_))
}

/// 1 ジョブ（Case × Attack × attempt）を実行する（DESIGN §5.5 / §11.3 / §11.4）．
async fn run_one_job<P: LlmProvider + Send + Sync + 'static>(ctx: JobCtx<P>) -> JobResult {
    // 有界並行: permit を取ってから生成に入る（終了までスコープ保持）．
    let _permit = ctx
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .expect("semaphore closed");

    let job_err = |kind: JobErrorKind| JobError {
        case: ctx.case.id.clone(),
        attack: ctx.attack.clone(),
        attempt: ctx.attempt,
        kind,
    };

    // 2. 変換適用後の最終プロンプトを作ってから生成（§6.2）．
    let rendered = match render(&ctx.case, &ctx.attack) {
        Ok(p) => p,
        Err(e) => return JobResult::Failed(job_err(JobErrorKind::Render(e.to_string()))),
    };

    // TPM 見積: プロンプト長 + 生成上限．
    let est_tokens =
        rendered.split_whitespace().count() as u32 + ctx.llm_config.max_tokens.unwrap_or(0);

    // 1./4. 多試行 seed は attempt から導出（§11.4）．リクエストは一定．
    let mut req = GenerateRequest::new(
        ctx.model.clone(),
        rendered,
        ctx.llm_config.clone(),
        ctx.attempt,
    );
    if let Some(n) = ctx.top_logprobs {
        req = req.with_top_logprobs(n);
    }
    let seed = req.effective_seed();

    // ジッタ RNG は送信 seed から決定論的に作る（§11.4 の explicit identity）．
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    // レート制御 + 429 バックオフ付きの生成ループ．
    let mut retry = 0u32;
    let output = loop {
        ctx.limiter.acquire(est_tokens).await;
        match ctx.provider.generate(&req).await {
            Ok(o) => break o,
            Err(e) => {
                if is_retryable(&e) && retry < ctx.max_retries {
                    let ra = parse_retry_after(&e.to_string());
                    let delay = ctx.backoff.delay(retry, ra, &mut rng);
                    tokio::time::sleep(delay).await;
                    retry += 1;
                    continue;
                }
                // リトライ上限まで回復せず → チェックポイントに刻まず失敗記録（次回再試行）．
                return JobResult::Failed(job_err(JobErrorKind::Provider(e.to_string())));
            }
        }
    };

    // 6. 1 応答に全測定器を当てる（§5.5）．
    let measurements: Vec<_> = ctx
        .instruments
        .iter()
        .map(|inst| inst.measure(&ctx.case, &output.response))
        .collect();

    let trial = Trial {
        case: ctx.case.id.clone(),
        attempt: ctx.attempt,
        model: ctx.model.clone(),
        attack: ctx.attack.clone(),
        response: output.response.clone(),
        measurements: measurements.clone(),
    };

    // 5. 完了を刻む: 1 生成 = 1 バッチ追記（生成の原子性・冪等，§11.3）．
    let records: Vec<CheckpointRecord> = measurements
        .into_iter()
        .map(|m| CheckpointRecord {
            case: ctx.case.id.clone(),
            attack: ctx.attack.clone(),
            attempt: ctx.attempt,
            seed,
            model: ctx.model.clone(),
            response: output.response.clone(),
            measurement: m,
        })
        .collect();

    if let Err(e) = ctx.checkpoint.append_records(records).await {
        // 生成は成功したが記録に失敗 → Trial を破棄して次回再生成させる（二重生成回避を優先）．
        return JobResult::Failed(job_err(JobErrorKind::Checkpoint(e.to_string())));
    }

    JobResult::Done(trial)
}
