//! ネットワーク非依存のモックプロバイダ（DESIGN §6.1 / IMPLEMENTATION_PLAN M4）．
//!
//! 既定（network-free）ビルドに含め，`CachingClient` と `pil-runner` をネットワーク無しで
//! テストできるようにする．応答は `(prompt, effective_seed, attempt)` から決定論的に生成する．

use std::sync::atomic::{AtomicUsize, Ordering};

use pil_core::{FinishReason, ModelRef, Response};

use crate::config::CallMetadata;
use crate::error::LlmError;
use crate::provider::{
    BoxFuture, GenerateOutput, GenerateRequest, LlmProvider, TokenLogprobs, ToolCall, TopLogprob,
};

/// 決定論的な canned 応答を返すモック（DESIGN §6.1）．
///
/// - 応答テキストは `(prompt, effective_seed, attempt)` から決まる（キャッシュ分離の検証用に，
///   attempt / prompt が違えば本文も変わる）．
/// - `generate` 呼び出し回数を数え，キャッシュ命中時に内側が呼ばれないことをテストで確認できる．
pub struct MockProvider {
    model: ModelRef,
    calls: AtomicUsize,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockProvider {
    /// 既定モデル `mock/mock-1`（endpoint 無し = network-free）で作る．
    pub fn new() -> Self {
        Self {
            model: ModelRef::new("mock", "mock-1", None),
            calls: AtomicUsize::new(0),
        }
    }

    /// `generate` が実際に呼ばれた回数（キャッシュ命中では増えない）．
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// このモックが名乗るモデル．
    pub fn model(&self) -> &ModelRef {
        &self.model
    }
}

impl LlmProvider for MockProvider {
    fn generate<'a>(
        &'a self,
        req: &'a GenerateRequest,
    ) -> BoxFuture<'a, Result<GenerateOutput, LlmError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);

            let seed = req.effective_seed();

            // ツールが広告され，かつ `tool_choice != "none"` なら決定論的にツール呼び出しを返す
            // （§4.1 / M2'．AgentDojo のエージェントループをネットワーク無しで駆動できるようにする）．
            let want_tool_call = req.tools.as_ref().is_some_and(|tools| !tools.is_empty())
                && req.tool_choice.as_deref() != Some("none");

            if want_tool_call {
                // 先頭ツールを決定論的に呼ぶ．`call_id` はツール名の blake3 先頭から導く．
                let tool = &req.tools.as_ref().unwrap()[0];
                let id = format!(
                    "call_{}",
                    &blake3::hash(tool.name.as_bytes()).to_hex()[..12]
                );
                let tool_call = ToolCall {
                    id,
                    name: tool.name.clone(),
                    arguments: "{}".to_string(),
                };

                let response = Response {
                    text: String::new(),
                    finish_reason: FinishReason::ToolCalls,
                    prompt_tokens: Some(req.prompt.split_whitespace().count() as u32),
                    completion_tokens: Some(0),
                    reached_clip_limit: false,
                };
                let metadata = CallMetadata {
                    model: req.model.model.clone(),
                    endpoint: req.model.endpoint.clone(),
                    temperature: req.config.temperature,
                    seed,
                    cache_hit: false,
                };

                return Ok(GenerateOutput {
                    response,
                    metadata,
                    logprobs: None,
                    tool_calls: vec![tool_call],
                });
            }

            // 決定論的な canned 応答．prompt / seed / attempt を織り込み，
            // 入力が違えば本文も変わるようにする．
            let text = format!(
                "MOCK response [seed={seed}, attempt={attempt}] to prompt: {prompt}",
                seed = seed,
                attempt = req.attempt,
                prompt = req.prompt,
            );
            let completion_tokens = text.split_whitespace().count() as u32;

            let response = Response {
                text,
                finish_reason: FinishReason::Stop,
                prompt_tokens: Some(req.prompt.split_whitespace().count() as u32),
                completion_tokens: Some(completion_tokens),
                reached_clip_limit: false,
            };

            let metadata = CallMetadata {
                model: req.model.model.clone(),
                endpoint: req.model.endpoint.clone(),
                temperature: req.config.temperature,
                seed,
                cache_hit: false,
            };

            // top_logprobs を要求されたら canned な分布を 1 トークン分だけ返す（§6.3）．
            let logprobs = req.top_logprobs.map(|n| {
                let top = (0..n.max(1))
                    .map(|i| TopLogprob {
                        token: format!("t{i}"),
                        logprob: -(i as f64) - 0.1,
                    })
                    .collect();
                vec![TokenLogprobs {
                    token: "t0".to_string(),
                    logprob: -0.1,
                    top,
                }]
            });

            Ok(GenerateOutput {
                response,
                metadata,
                logprobs,
                tool_calls: Vec::new(),
            })
        })
    }
}
