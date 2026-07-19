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
    /// エージェントが広告するツール群（M2'．`{type:"function", function:{...}}` の配列）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ChatCompletionTool>>,
    /// ツール選択方針（M2'．文字列 `"auto"`/`"none"`/`"required"` またはオブジェクト）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
}

/// OpenAI の 1 ツール定義（`{type:"function", function:{name, description, parameters}}`．M2'）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletionTool {
    /// 常に `"function"`
    #[serde(rename = "type")]
    pub tool_type: String,
    /// 関数の仕様
    pub function: FunctionDef,
}

/// `tools[].function`（`{name, description, parameters}`．`parameters` は JSON Schema．M2'）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDef {
    /// 関数名
    pub name: String,
    /// 関数の説明（省略可）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 引数の JSON Schema（省略時は空オブジェクト相当）
    #[serde(default)]
    pub parameters: serde_json::Value,
}

/// 1 メッセージ（`role` と `content`，及びツール呼び出し関連．M2'）．
///
/// AgentDojo はエージェントループで多様な役割を送ってくる:
/// - `role="developer"`（system 相当）／`role="user"`／`role="assistant"`
/// - `role="assistant"` ＋ `tool_calls`（前ターンでモデルが要求したツール呼び出し）
/// - `role="tool"` ＋ `tool_call_id`（そのツールの実行結果）
///
/// これらを表現できるよう `content` は `Option`（tool_calls 応答時は null）とし，
/// `tool_calls` / `tool_call_id` / `name` を任意フィールドで持つ（欠損時は素通し可能）．
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    /// `"system"` / `"developer"` / `"user"` / `"assistant"` / `"tool"` 等
    pub role: String,
    /// 本文．`tool_calls` を伴う assistant 応答では null（省略）になり得る
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// assistant がこのターンで要求したツール呼び出し（OpenAI 形．無ければ空で省略）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallObject>,
    /// `role="tool"` メッセージが対応付ける呼び出し ID（結果 → 呼び出しの紐付け）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// ツール名（`role="tool"` の結果メッセージ等が持つことがある）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// OpenAI `tool_calls[]` の 1 要素（`{id, type:"function", function:{name, arguments}}`．M2'）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallObject {
    /// 呼び出し ID（`call_...`）
    pub id: String,
    /// 常に `"function"`
    #[serde(rename = "type")]
    pub call_type: String,
    /// 呼ぶ関数（名前と引数 JSON 文字列）
    pub function: FunctionCall,
}

/// `tool_calls[].function`（`{name, arguments}`．`arguments` は生 JSON 文字列．M2'）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    /// 関数名
    pub name: String,
    /// 引数の生 JSON 文字列（OpenAI 準拠）
    pub arguments: String,
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
