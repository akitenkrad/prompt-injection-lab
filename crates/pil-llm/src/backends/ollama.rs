//! Ollama バックエンド（DESIGN §6.3 / §11.1）．feature = "ollama"．
//!
//! - **起動時に `>= 0.12.11` を検査**し，満たさなければ明示失敗する（§6.3．judge を黙って
//!   劣化させないため）．
//! - `top_logprobs` は native `/api/generate` で取得する（§6.3．native は確定，OpenAI 互換
//!   経路での実返却は Phase 2 で実測）．
//!
//! このモジュールは feature gate 下でのみコンパイルされ，既定ビルドは reqwest を参照しない．

use serde::{Deserialize, Serialize};

use pil_core::{FinishReason, Response};

use crate::config::CallMetadata;
use crate::error::LlmError;
use crate::provider::{
    BoxFuture, GenerateOutput, GenerateRequest, LlmProvider, TokenLogprobs, TopLogprob,
};
use crate::version::{meets_minimum, min_version_string, OLLAMA_MIN_VERSION};

/// Ollama プロバイダ．`connect` 時にバージョンを検査済み．
pub struct OllamaProvider {
    client: reqwest::Client,
    /// 例: `"http://localhost:11434"`（末尾スラッシュ無し）
    endpoint: String,
}

impl OllamaProvider {
    /// エンドポイントに接続し，バージョン要件（§6.3）を検査する．
    ///
    /// `>= 0.12.11` を満たさなければ `LlmError::UnsupportedVersion` を返す．
    pub async fn connect(endpoint: impl Into<String>) -> Result<Self, LlmError> {
        let endpoint = endpoint.into().trim_end_matches('/').to_string();
        let client = reqwest::Client::new();
        let version = fetch_version(&client, &endpoint).await?;
        if !meets_minimum(&version, OLLAMA_MIN_VERSION) {
            return Err(LlmError::UnsupportedVersion {
                found: version,
                required: min_version_string(),
            });
        }
        Ok(Self { client, endpoint })
    }

    /// エンドポイント文字列．
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// `/api/version` を叩いてバージョン文字列を得る（§11.1）．
async fn fetch_version(client: &reqwest::Client, endpoint: &str) -> Result<String, LlmError> {
    let url = format!("{endpoint}/api/version");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(LlmError::Provider(format!(
            "GET /api/version returned {}",
            resp.status()
        )));
    }
    let body: VersionResp = resp
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;
    Ok(body.version)
}

#[derive(Deserialize)]
struct VersionResp {
    version: String,
}

// --- /api/generate リクエスト／レスポンス ---

#[derive(Serialize)]
struct GenerateReqBody<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    options: GenerateOptions,
    /// logprobs を有効化（§6.3）
    #[serde(skip_serializing_if = "Option::is_none")]
    logprobs: Option<bool>,
    /// 各位置の上位候補数（§6.3）
    #[serde(skip_serializing_if = "Option::is_none")]
    top_logprobs: Option<u32>,
}

#[derive(Serialize)]
struct GenerateOptions {
    temperature: f32,
    seed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

#[derive(Deserialize, Default)]
struct GenerateRespBody {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
    #[serde(default)]
    logprobs: Option<Vec<OllamaTokenLogprob>>,
}

#[derive(Deserialize)]
struct OllamaTokenLogprob {
    #[serde(default)]
    token: String,
    #[serde(default)]
    logprob: f64,
    #[serde(default)]
    top_logprobs: Vec<OllamaTop>,
}

#[derive(Deserialize)]
struct OllamaTop {
    #[serde(default)]
    token: String,
    #[serde(default)]
    logprob: f64,
}

fn map_finish_reason(done_reason: Option<&str>) -> FinishReason {
    match done_reason {
        Some("stop") | None => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some(other) => FinishReason::Other(other.to_string()),
    }
}

fn map_logprobs(raw: Option<Vec<OllamaTokenLogprob>>) -> Option<Vec<TokenLogprobs>> {
    raw.map(|tokens| {
        tokens
            .into_iter()
            .map(|t| TokenLogprobs {
                token: t.token,
                logprob: t.logprob,
                top: t
                    .top_logprobs
                    .into_iter()
                    .map(|c| TopLogprob {
                        token: c.token,
                        logprob: c.logprob,
                    })
                    .collect(),
            })
            .collect()
    })
}

impl LlmProvider for OllamaProvider {
    fn generate<'a>(
        &'a self,
        req: &'a GenerateRequest,
    ) -> BoxFuture<'a, Result<GenerateOutput, LlmError>> {
        Box::pin(async move {
            let seed = req.effective_seed();
            let want_logprobs = req.top_logprobs.is_some();
            let body = GenerateReqBody {
                model: &req.model.model,
                prompt: &req.prompt,
                stream: false,
                system: req.config.system.as_deref(),
                options: GenerateOptions {
                    temperature: req.config.temperature,
                    seed,
                    num_predict: req.config.max_tokens,
                },
                logprobs: want_logprobs.then_some(true),
                top_logprobs: req.top_logprobs,
            };

            let url = format!("{}/api/generate", self.endpoint);
            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Network(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(LlmError::Provider(format!(
                    "POST /api/generate returned {}",
                    resp.status()
                )));
            }
            let parsed: GenerateRespBody = resp
                .json()
                .await
                .map_err(|e| LlmError::Parse(e.to_string()))?;

            let finish_reason = map_finish_reason(parsed.done_reason.as_deref());
            let response = Response {
                text: parsed.response,
                finish_reason,
                prompt_tokens: parsed.prompt_eval_count,
                completion_tokens: parsed.eval_count,
                reached_clip_limit: false,
            };
            let metadata = CallMetadata {
                model: req.model.model.clone(),
                endpoint: Some(self.endpoint.clone()),
                temperature: req.config.temperature,
                seed,
                cache_hit: false,
            };

            Ok(GenerateOutput {
                response,
                metadata,
                logprobs: map_logprobs(parsed.logprobs),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(map_finish_reason(Some("stop")), FinishReason::Stop);
        assert_eq!(map_finish_reason(None), FinishReason::Stop);
        assert_eq!(map_finish_reason(Some("length")), FinishReason::Length);
        assert_eq!(
            map_finish_reason(Some("load")),
            FinishReason::Other("load".to_string())
        );
    }

    #[test]
    fn version_gate_rejects_old() {
        // connect はネットワークを要するが，判定ロジック（version モジュール）は純粋にテスト済み．
        assert!(!meets_minimum("0.12.10", OLLAMA_MIN_VERSION));
        assert!(meets_minimum("0.12.11", OLLAMA_MIN_VERSION));
    }
}
