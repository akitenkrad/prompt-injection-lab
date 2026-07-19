//! `pil-runner` のエラー型（DESIGN §11.3）．
//!
//! - [`RunnerError`] — 実行全体を止める致命的失敗（チェックポイント I/O 等）．`run` の `Err` 側．
//! - [`JobError`] — 1 ジョブ（Case × Attack × attempt）だけが失敗した場合．全体は続行し，
//!   `RunOutcome.errors` に集約する．**プロバイダ失敗のジョブはチェックポイントに刻まない**ため，
//!   次回起動で再試行される（§11.3 の中断再開・冪等性と両立）．

use pil_core::{AttackRef, CaseId};

/// 実行全体を止める致命的失敗．
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// チェックポイントファイルの読み書き失敗．
    #[error("checkpoint I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// チェックポイント JSONL のシリアライズ／デシリアライズ失敗．
    #[error("checkpoint (de)serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// 並行実行タスクの join 失敗（パニック等）．
    #[error("worker task join error: {0}")]
    Join(String),
}

/// 1 ジョブだけが失敗したときの記録（全体は続行する）．
#[derive(Debug, Clone, PartialEq)]
pub struct JobError {
    pub case: CaseId,
    pub attack: AttackRef,
    pub attempt: u32,
    pub kind: JobErrorKind,
}

/// [`JobError`] の種別．
#[derive(Debug, Clone, PartialEq)]
pub enum JobErrorKind {
    /// `pil_attacks::render` が失敗（未対応言語・未知テンプレート等）．生成には至らない．
    Render(String),
    /// プロバイダがリトライ上限まで失敗した（§11.3 のバックオフ後も回復せず）．
    /// この場合チェックポイントに刻まないため次回再試行される．
    Provider(String),
    /// 生成は成功したがチェックポイント追記に失敗した．冪等性のため Trial は破棄し，
    /// 次回起動で再生成させる（二重生成を避けるより，未記録の再生成を選ぶ）．
    Checkpoint(String),
}
