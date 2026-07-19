//! 三値判定（DESIGN §5.3 / §3.6）．
//!
//! 判定不能を 0 に潰せば ASR は下振れし，成功に潰せば上振れする．どちらも報告されない．
//! よって判定は三値 `Success | Failure | Undecidable{reason}` とし，二値化は `pil-report` の
//! 明示操作でのみ行い，潰した件数を必ず併記する（§5.3）．

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Verdict {
    Success,
    Failure,
    /// 成功にも失敗にも還元できない．0 に潰すと ASR が下振れする．
    Undecidable {
        reason: UndecidableReason,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UndecidableReason {
    /// HarmBench: 分類器が yes/no 以外を返した（上流の `-1` に相当）
    ClassifierOutOfVocab { raw: String },
    /// StrongREJECT v1: ルーブリック正規表現に一致しない（上流の `NaN` に相当）
    ParseFailure { raw: String },
    /// 応答がクリップ長に達しており，判定が信頼できない
    ResponseTruncated,
    /// プロバイダ側のエラー（レート制限・タイムアウト等）
    ProviderError { message: String },
    /// この Case にこの測定器は適用できない（例: target 列が無い AdvBench に拒否マッチ）
    NotApplicable { reason: String },
}

impl Verdict {
    pub fn is_success(&self) -> bool {
        matches!(self, Verdict::Success)
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Verdict::Failure)
    }

    pub fn is_undecidable(&self) -> bool {
        matches!(self, Verdict::Undecidable { .. })
    }
}
