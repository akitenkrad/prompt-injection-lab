//! 測定器の同一性（DESIGN §5.4）— §3.7（StrongREJECT v1/v2 問題）への直接の回答．

use serde::{Deserialize, Serialize};

use crate::model::ModelRef;
use crate::source::SourceRef;
use crate::verdict::Verdict;

/// 測定器の同一性．同名別実体（rubric v1/v2 等）を `version` と `source` で区別する．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentRef {
    /// 例: `"strongreject-rubric"`
    pub name: String,
    /// 例: `"v1"` | `"v2"` — 同名別実体を区別する
    pub version: String,
    /// プロンプト/分類器がどの commit のどのファイル由来か
    pub source: SourceRef,
    pub params: MeasurementParams,
}

/// §3.9: ASR を動かすのに通常報告されないパラメータ．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasurementParams {
    /// HarmBench: 512, StrongREJECT finetuned: 512
    pub response_clip_tokens: Option<u32>,
    /// LLM judge の場合のみ
    pub judge_model: Option<ModelRef>,
    pub temperature: f32,
}

/// 1 件の測定結果．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub verdict: Verdict,
    /// StrongREJECT の `[0,1]` スコア等．二値判定器では None
    pub score: Option<f64>,
    pub instrument: InstrumentRef,
    /// 判定器の生出力．事後の再解析のため必ず保持する
    pub raw: String,
}
