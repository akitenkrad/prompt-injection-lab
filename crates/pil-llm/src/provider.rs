//! プロバイダ抽象（DESIGN §6.1 / §6.3 / §4.1）．
//!
//! 生成を担う `LlmProvider` トレイトと，その入出力型．`top_logprobs` を返せる API を型として
//! 定義する（§6.3 の fine-tuned judge が要求．Phase 1 の実配線は Ollama native 経路のみ）．
//! トレイトは **object-safe** に保ち，`pil-runner` が `dyn LlmProvider` として注入できるようにする．

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use pil_core::{ModelRef, Response};

use crate::config::{seed_for_attempt, CallMetadata, LlmConfig};
use crate::error::LlmError;

/// object-safe な非同期戻り値．`async fn in trait` は dyn 非互換のため，
/// 戻り値をボックス化した Future にしてトレイトオブジェクト化を可能にする．
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// エージェント（AgentDojo 等）が広告する 1 ツールの仕様（DESIGN §4.1 / M2'）．
///
/// OpenAI の `tools[].function`（`{name, description, parameters}`）に対応する中立表現．
/// `parameters` はツール引数の JSON Schema をそのまま保持する（シムを素通しするため）．
/// ツール情報を**副経路ではなく** `GenerateRequest` 単一経路に集約する（§4.1 の単一経路原則）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// ツール名（関数名）
    pub name: String,
    /// ツールの説明（省略可）
    pub description: Option<String>,
    /// 引数の JSON Schema（そのまま保持）
    pub parameters: serde_json::Value,
}

/// モデルが要求した 1 ツール呼び出し（DESIGN §4.1 / M2'）．
///
/// `arguments` は OpenAI 同様に**生の JSON 文字列**として保持する（構造化しない）．
/// 応答側では `choices[0].message.tool_calls[]` の `function.arguments` に素通しする．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// 呼び出し ID（`call_...`）．後続の `role="tool"` 結果と対応付ける
    pub id: String,
    /// 呼ぶツール名（関数名）
    pub name: String,
    /// 引数の生 JSON 文字列（OpenAI 準拠）
    pub arguments: String,
}

/// 1 生成の要求（DESIGN §6.2）．
///
/// `prompt` は `AttackRef` の変換適用後の**最終送信プロンプト**（`rendered_prompt`, §5.6 / §6.2）．
/// `attempt` と `config.seed` から実送信 seed が定まる（§11.4）．
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateRequest {
    /// 呼び出すモデル
    pub model: ModelRef,
    /// 変換適用後の最終送信プロンプト（rendered_prompt）
    pub prompt: String,
    /// 温度・基底 seed・上限・system
    pub config: LlmConfig,
    /// 多試行 ASR 用の試行番号（1..=100）．キャッシュキーと seed に効く（§6.2 / §11.4）
    pub attempt: u32,
    /// `top_logprobs` を要求する場合の個数（None なら要求しない．§6.3）
    pub top_logprobs: Option<u32>,
    /// エージェントが広告するツール群（None なら通常の生成．§4.1 / M2'）
    pub tools: Option<Vec<ToolSpec>>,
    /// ツール選択方針（`"auto"` / `"none"` / `"required"` 等．OpenAI の `tool_choice` 文字列）
    pub tool_choice: Option<String>,
}

impl GenerateRequest {
    pub fn new(
        model: ModelRef,
        prompt: impl Into<String>,
        config: LlmConfig,
        attempt: u32,
    ) -> Self {
        Self {
            model,
            prompt: prompt.into(),
            config,
            attempt,
            top_logprobs: None,
            tools: None,
            tool_choice: None,
        }
    }

    /// `top_logprobs` の要求個数を設定する（§6.3）．
    pub fn with_top_logprobs(mut self, n: u32) -> Self {
        self.top_logprobs = Some(n);
        self
    }

    /// 広告ツール群を設定する（§4.1 / M2'）．
    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// ツール選択方針を設定する（§4.1 / M2'）．
    pub fn with_tool_choice(mut self, tool_choice: String) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    /// この要求の実送信 seed（`seed_for_attempt(config.seed, attempt)`, §11.4）．
    pub fn effective_seed(&self) -> u64 {
        seed_for_attempt(self.config.seed, self.attempt)
    }
}

/// 生成の出力．`pil_core::Response` に呼び出しメタデータと（要求時）logprobs を添える．
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateOutput {
    /// 生成応答本体（§5.5）
    pub response: Response,
    /// 何と話したか（§6.1）
    pub metadata: CallMetadata,
    /// トークン毎の logprobs（§6.3．要求時かつプロバイダが返した場合のみ Some）
    pub logprobs: Option<Vec<TokenLogprobs>>,
    /// モデルが要求したツール呼び出し（無ければ空．§4.1 / M2'）
    pub tool_calls: Vec<ToolCall>,
}

/// 生成された 1 トークンの logprob と，その位置の候補分布（DESIGN §6.3 / §8.3）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenLogprobs {
    /// 実際に選ばれたトークン
    pub token: String,
    /// 選ばれたトークンの log 確率
    pub logprob: f64,
    /// 同位置の上位候補（`top_logprobs`）．fine-tuned judge の期待値式が参照する
    pub top: Vec<TopLogprob>,
}

/// `top_logprobs` の 1 候補．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopLogprob {
    pub token: String,
    pub logprob: f64,
}

/// 生成プロバイダ（DESIGN §6.1）．
///
/// **object-safe** を維持する（型パラメタを取らずライフタイムのみ）ため，`pil-runner` は
/// `Box<dyn LlmProvider>` / `Arc<dyn LlmProvider>` として注入できる（§4.1）．
pub trait LlmProvider: Send + Sync {
    /// `req` に従って生成する．失敗は `LlmError`．
    fn generate<'a>(
        &'a self,
        req: &'a GenerateRequest,
    ) -> BoxFuture<'a, Result<GenerateOutput, LlmError>>;
}

// トレイトオブジェクト（Box / Arc）越しでもそのまま `LlmProvider` として扱えるようにする．
// これにより `CachingClient<Box<dyn LlmProvider>>` 等が成立する．

impl<T: LlmProvider + ?Sized> LlmProvider for Box<T> {
    fn generate<'a>(
        &'a self,
        req: &'a GenerateRequest,
    ) -> BoxFuture<'a, Result<GenerateOutput, LlmError>> {
        (**self).generate(req)
    }
}

impl<T: LlmProvider + ?Sized> LlmProvider for Arc<T> {
    fn generate<'a>(
        &'a self,
        req: &'a GenerateRequest,
    ) -> BoxFuture<'a, Result<GenerateOutput, LlmError>> {
        (**self).generate(req)
    }
}
