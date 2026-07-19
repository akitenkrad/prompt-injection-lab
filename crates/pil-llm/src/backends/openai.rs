//! OpenAI バックエンド骨組み（DESIGN §6.1 / §4.1）．feature = "openai"．
//!
//! Phase 1 では必須でない骨組み．OpenAI 互換 REST 面（`<base_url>/chat/completions`, §11.1）を
//! 通す実装は Phase 2 で行う．現状は `LlmError::NotImplemented` を返す（feature 有効時に
//! コンパイルは通る）．

use crate::error::LlmError;
use crate::provider::{BoxFuture, GenerateOutput, GenerateRequest, LlmProvider};

/// OpenAI（互換）プロバイダの骨組み．
#[allow(dead_code)]
pub struct OpenAiProvider {
    client: reqwest::Client,
    /// 例: `"https://api.openai.com/v1"`
    endpoint: String,
    api_key: String,
}

impl OpenAiProvider {
    pub fn new(endpoint: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
        }
    }
}

impl LlmProvider for OpenAiProvider {
    fn generate<'a>(
        &'a self,
        _req: &'a GenerateRequest,
    ) -> BoxFuture<'a, Result<GenerateOutput, LlmError>> {
        Box::pin(async move {
            Err(LlmError::NotImplemented(format!(
                "openai backend ({}) is a Phase 2 skeleton",
                self.endpoint
            )))
        })
    }
}
