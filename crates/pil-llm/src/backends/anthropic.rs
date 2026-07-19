//! Anthropic バックエンド骨組み（DESIGN §6.1 / §4.1）．feature = "anthropic"．
//!
//! Phase 1 では必須でない骨組み．tool-calling スキーマの OpenAI 形式への翻訳（§4.1）を含む
//! 実装は Phase 2 で行う．現状は `LlmError::NotImplemented` を返す．

use crate::error::LlmError;
use crate::provider::{BoxFuture, GenerateOutput, GenerateRequest, LlmProvider};

/// Anthropic プロバイダの骨組み．
#[allow(dead_code)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    /// 例: `"https://api.anthropic.com"`
    endpoint: String,
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(endpoint: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
        }
    }
}

impl LlmProvider for AnthropicProvider {
    fn generate<'a>(
        &'a self,
        _req: &'a GenerateRequest,
    ) -> BoxFuture<'a, Result<GenerateOutput, LlmError>> {
        Box::pin(async move {
            Err(LlmError::NotImplemented(format!(
                "anthropic backend ({}) is a Phase 2 skeleton",
                self.endpoint
            )))
        })
    }
}
