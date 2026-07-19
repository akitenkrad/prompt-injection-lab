//! OpenAI ChatCompletion 互換の serde 型（DESIGN §4.1）．
//!
//! これらの型は **ネットワーク非依存**であり，`shim` feature 無しでも参照・単体テストできる．
//! HTTP サーバ（[`crate::server`]）はこの型を axum の `Json` 抽出／応答に用いるだけで，
//! 変換ロジック自体は [`crate::mapping`] の純関数が担う（§4.1 の制御反転）．

use serde::{Deserialize, Serialize};

/// OpenAI `POST /v1/chat/completions` の要求本体（非ストリーミング，M1 範囲）．
///
/// `stream` は受理するが M1 では非ストリーミングのみ実装する（§4.1）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    /// 呼び出すモデル．`"provider/model"` 形式なら provider を割り出す（[`crate::mapping::model_ref_from_model`]）
    pub model: String,
    /// 会話メッセージ列．`system` は `LlmConfig.system` へ，それ以外は rendered prompt へ畳む
    pub messages: Vec<ChatMessage>,
    /// サンプリング温度（省略時は `LlmConfig` 既定＝決定論の 0.0）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// 基底 seed（§11.4．省略時は既定 0）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// 生成上限トークン数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// logprobs を要求するか（§6.3）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    /// 各位置の上位候補数（§6.3．`GenerateRequest.top_logprobs` に写る）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
    /// ストリーミング要求（M1 では非対応・受理のみ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

/// 1 メッセージ（`role` と `content`）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// `"system"` / `"user"` / `"assistant"` / `"tool"` 等
    pub role: String,
    /// 本文（欠損時は空文字）
    #[serde(default)]
    pub content: String,
}

/// OpenAI ChatCompletion の応答本体．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    /// `chatcmpl-...`（応答本文から決定論的に導出）
    pub id: String,
    /// 常に `"chat.completion"`
    pub object: String,
    /// 生成時刻（純変換では決定論のため 0．サーバ側で上書きしない）
    pub created: u64,
    /// 要求で指定されたモデル名をそのまま反映
    pub model: String,
    /// 生成候補（M1 は常に 1 件）
    pub choices: Vec<ChatChoice>,
    /// トークン使用量（プロバイダが返した場合のみ）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

/// 応答の 1 候補．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatChoice {
    /// 候補番号（M1 は 0 のみ）
    pub index: u32,
    /// `role="assistant"` のメッセージ
    pub message: ChatMessage,
    /// 停止理由（`FinishReason` から写像）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// logprobs（要求時かつプロバイダが返した場合のみ．§6.3）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<ChoiceLogprobs>,
}

/// OpenAI `choices[].logprobs` 形（`content[]` にトークン毎の logprob を並べる．§6.3）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceLogprobs {
    /// 生成トークン列の logprob
    pub content: Vec<TokenLogprobEntry>,
}

/// 生成 1 トークンの logprob とその位置の上位候補．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenLogprobEntry {
    /// 実際に選ばれたトークン
    pub token: String,
    /// その log 確率
    pub logprob: f64,
    /// トークンのバイト列（M1 では出力しない）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    /// 同位置の上位候補
    pub top_logprobs: Vec<TopLogprobEntry>,
}

/// `top_logprobs` の 1 候補．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopLogprobEntry {
    /// 候補トークン
    pub token: String,
    /// その log 確率
    pub logprob: f64,
    /// トークンのバイト列（M1 では出力しない）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

/// トークン使用量．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// プロンプト側トークン数
    pub prompt_tokens: u32,
    /// 生成側トークン数
    pub completion_tokens: u32,
    /// 合計
    pub total_tokens: u32,
}

/// OpenAI 形式のエラー応答（`{"error": {...}}`）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiErrorResponse {
    /// エラー本体
    pub error: OpenAiError,
}

/// OpenAI 形式のエラー本体．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiError {
    /// 人間可読なメッセージ（`LlmError` の Display）
    pub message: String,
    /// エラー種別（`"not_implemented"` 等）
    #[serde(rename = "type")]
    pub error_type: String,
    /// エラーコード（M1 では常に None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 関連パラメタ名（M1 では常に None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
}

/// `GET /v1/models` の応答．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelList {
    /// 常に `"list"`
    pub object: String,
    /// モデル一覧
    pub data: Vec<ModelObject>,
}

/// モデル 1 件のメタデータ．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelObject {
    /// モデル ID
    pub id: String,
    /// 常に `"model"`
    pub object: String,
    /// 作成時刻（決定論のため 0）
    pub created: u64,
    /// 所有者（本シムでは `"pil-shim"`）
    pub owned_by: String,
}
