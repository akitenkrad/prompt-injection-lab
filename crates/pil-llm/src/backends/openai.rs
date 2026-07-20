//! OpenAI 互換バックエンド（DESIGN §6.1 / §4.1 / §11.1）．feature = "openai"．
//!
//! `<endpoint>/chat/completions`（endpoint は `/v1` を含んで構築される）へ POST し，
//! `Authorization: Bearer <api_key>` を付す．ツール呼び出し（tools/tool_calls）と logprobs を
//! 素通しする（M2'）．ローカル Ollama の OpenAI 互換面（`http://localhost:11434/v1`）にも接続できる．
//!
//! リクエスト構築（[`build_chat_request`]）と応答解釈（[`parse_chat_response`]）は
//! **ネットワーク非依存の純関数**に括り出し，HTTP 無しで単体テストできる（§4.1 の制御反転）．
//! 送受信の serde 形状は `pil-shim` の OpenAI 型を鏡写しにしたプロバイダ局所の型で表現する
//! （crate 間依存は張らない）．
//!
//! このモジュールは feature gate 下でのみコンパイルされ，既定ビルドは reqwest を参照しない．

use serde::{Deserialize, Serialize};
use serde_json::json;

use pil_core::{FinishReason, Response};

use crate::config::CallMetadata;
use crate::error::LlmError;
use crate::provider::{
    BoxFuture, GenerateOutput, GenerateRequest, LlmProvider, TokenLogprobs, ToolCall, TopLogprob,
};

/// OpenAI（互換）プロバイダ．
pub struct OpenAiProvider {
    client: reqwest::Client,
    /// 例: `"https://api.openai.com/v1"` / `"http://localhost:11434/v1"`（`/v1` を含む）
    endpoint: String,
    api_key: String,
    /// 送信モデル名の上書き（§4.1）．`Some` なら要求のモデル名に依らずこの値を送る．
    ///
    /// 上流ベンチ（AgentDojo 等）が固定の enum モデル名しか受け付けない一方，実プロバイダ
    /// （ローカル Ollama）は別タグを要する場合に，シム境界でモデル名を実タグへ差し替える．
    model_override: Option<String>,
}

impl OpenAiProvider {
    /// `endpoint`（`/v1` を含む）と API 鍵からプロバイダを作る．
    pub fn new(endpoint: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model_override: None,
        }
    }

    /// 送信モデル名を上書きする（§4.1．要求のモデル名に依らずこの値を実プロバイダへ送る）．
    pub fn with_model_override(mut self, model: impl Into<String>) -> Self {
        self.model_override = Some(model.into());
        self
    }

    /// エンドポイント文字列（`/v1` を含む，末尾スラッシュ無し）．
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

// --- 送信 serde 形状（pil-shim の OpenAI 要求型を鏡写し．crate 間依存は張らない） ---

/// `POST /v1/chat/completions` の要求本体（非ストリーミング）．
#[derive(Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: Vec<ReqMessage<'a>>,
    temperature: f32,
    seed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ReqTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_logprobs: Option<u32>,
}

/// 1 メッセージ（system / user のみ送る）．
#[derive(Serialize)]
struct ReqMessage<'a> {
    role: &'a str,
    content: &'a str,
}

/// `tools[]` の 1 要素（`{type:"function", function:{...}}`）．
#[derive(Serialize)]
struct ReqTool<'a> {
    #[serde(rename = "type")]
    tool_type: &'a str,
    function: ReqFunction<'a>,
}

/// `tools[].function`（`{name, description?, parameters}`）．
#[derive(Serialize)]
struct ReqFunction<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    parameters: &'a serde_json::Value,
}

/// OpenAI ChatCompletion 要求 JSON を組み立てる純関数（§4.1）．
///
/// - `messages` = （`config.system` があれば system 1 件）＋ user 1 件（`req.prompt`）．
/// - `temperature`/`seed` は常に載せ，`max_tokens`/`tools`/`tool_choice` は指定時のみ載せる．
/// - `req.top_logprobs` があれば `logprobs=true` と `top_logprobs` を付す（§6.3）．
pub fn build_chat_request(req: &GenerateRequest) -> serde_json::Value {
    let mut messages = Vec::new();
    if let Some(system) = req.config.system.as_deref() {
        messages.push(ReqMessage {
            role: "system",
            content: system,
        });
    }
    messages.push(ReqMessage {
        role: "user",
        content: &req.prompt,
    });

    let tools = req.tools.as_ref().map(|tools| {
        tools
            .iter()
            .map(|tool| ReqTool {
                tool_type: "function",
                function: ReqFunction {
                    name: &tool.name,
                    description: tool.description.as_deref(),
                    parameters: &tool.parameters,
                },
            })
            .collect::<Vec<_>>()
    });

    let want_logprobs = req.top_logprobs.is_some();
    let chat = ChatReq {
        model: &req.model.model,
        messages,
        temperature: req.config.temperature,
        seed: req.effective_seed(),
        max_tokens: req.config.max_tokens,
        tools,
        tool_choice: req.tool_choice.as_deref(),
        logprobs: want_logprobs.then_some(true),
        top_logprobs: req.top_logprobs,
    };

    // これらの型は必ず JSON に落ちる（失敗は論理的に起こらない）．
    serde_json::to_value(&chat).unwrap_or_else(|_| json!({}))
}

// --- 受信 serde 形状（pil-shim の OpenAI 応答型を鏡写し） ---

/// ChatCompletion 応答本体．
#[derive(Deserialize)]
struct ChatResp {
    #[serde(default)]
    choices: Vec<RespChoice>,
    #[serde(default)]
    usage: Option<RespUsage>,
}

/// 応答の 1 候補．
#[derive(Deserialize, Default)]
struct RespChoice {
    #[serde(default)]
    message: RespMessage,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    logprobs: Option<RespLogprobs>,
}

/// `choices[].message`（`content` は tool_calls 時 null になり得る）．
#[derive(Deserialize, Default)]
struct RespMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<RespToolCall>,
}

/// `tool_calls[]` の 1 要素（`{id, function:{name, arguments}}`）．
#[derive(Deserialize, Default)]
struct RespToolCall {
    #[serde(default)]
    id: String,
    #[serde(default)]
    function: RespFunctionCall,
}

/// `tool_calls[].function`（`{name, arguments}`．`arguments` は生 JSON 文字列）．
#[derive(Deserialize, Default)]
struct RespFunctionCall {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

/// トークン使用量．
#[derive(Deserialize)]
struct RespUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
}

/// `choices[].logprobs`（`content[]` にトークン毎の logprob）．
#[derive(Deserialize)]
struct RespLogprobs {
    #[serde(default)]
    content: Vec<RespTokenLogprob>,
}

/// 生成 1 トークンの logprob とその位置の上位候補．
#[derive(Deserialize)]
struct RespTokenLogprob {
    #[serde(default)]
    token: String,
    #[serde(default)]
    logprob: f64,
    #[serde(default)]
    top_logprobs: Vec<RespTopLogprob>,
}

/// `top_logprobs` の 1 候補．
#[derive(Deserialize)]
struct RespTopLogprob {
    #[serde(default)]
    token: String,
    #[serde(default)]
    logprob: f64,
}

/// [`parse_chat_response`] の純出力（metadata は要求側の情報から呼び出し側で組む）．
struct ParsedResponse {
    response: Response,
    tool_calls: Vec<ToolCall>,
    logprobs: Option<Vec<TokenLogprobs>>,
}

/// OpenAI の `finish_reason` を [`FinishReason`] に写す．
fn map_finish_reason(raw: Option<&str>) -> FinishReason {
    match raw {
        Some("stop") | None => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some("tool_calls") => FinishReason::ToolCalls,
        Some(other) => FinishReason::Other(other.to_string()),
    }
}

/// `choices[].logprobs.content[]` を [`TokenLogprobs`] へ写す（§6.3）．
fn map_logprobs(raw: &RespLogprobs) -> Vec<TokenLogprobs> {
    raw.content
        .iter()
        .map(|token| TokenLogprobs {
            token: token.token.clone(),
            logprob: token.logprob,
            top: token
                .top_logprobs
                .iter()
                .map(|candidate| TopLogprob {
                    token: candidate.token.clone(),
                    logprob: candidate.logprob,
                })
                .collect(),
        })
        .collect()
}

/// ChatCompletion 応答を `pil-llm` の中立表現に写す純関数（§4.1 / §6.3）．
///
/// - `choices[0].message.content`（null 可）→ `Response.text`（null は空文字）．
/// - `choices[0].message.tool_calls[]` → `Vec<ToolCall>`（id / name / arguments）．
/// - `choices[0].finish_reason` → [`FinishReason`]．`usage` → prompt/completion tokens．
/// - `choices[0].logprobs.content[]` → `Vec<TokenLogprobs>`（あれば）．
fn parse_chat_response(resp: &ChatResp) -> ParsedResponse {
    let choice = resp.choices.first();
    let message = choice.map(|c| &c.message);

    let text = message.and_then(|m| m.content.clone()).unwrap_or_default();

    let tool_calls: Vec<ToolCall> = message
        .map(|m| {
            m.tool_calls
                .iter()
                .map(|call| ToolCall {
                    id: call.id.clone(),
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let finish_reason = map_finish_reason(choice.and_then(|c| c.finish_reason.as_deref()));

    let (prompt_tokens, completion_tokens) = match resp.usage.as_ref() {
        Some(usage) => (usage.prompt_tokens, usage.completion_tokens),
        None => (None, None),
    };

    let logprobs = choice.and_then(|c| c.logprobs.as_ref()).map(map_logprobs);

    let response = Response {
        text,
        finish_reason,
        prompt_tokens,
        completion_tokens,
        reached_clip_limit: false,
    };

    ParsedResponse {
        response,
        tool_calls,
        logprobs,
    }
}

impl LlmProvider for OpenAiProvider {
    fn generate<'a>(
        &'a self,
        req: &'a GenerateRequest,
    ) -> BoxFuture<'a, Result<GenerateOutput, LlmError>> {
        Box::pin(async move {
            let seed = req.effective_seed();
            let mut body = build_chat_request(req);
            // 送信モデル名の上書き（§4.1．上流の enum モデル名を実プロバイダのタグへ差し替える）．
            if let Some(model) = self.model_override.as_deref() {
                body["model"] = serde_json::Value::String(model.to_string());
            }

            let url = format!("{}/chat/completions", self.endpoint);
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Network(e.to_string()))?;

            let status = resp.status();
            if !status.is_success() {
                let snippet: String = resp
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(500)
                    .collect();
                return Err(LlmError::Provider(format!(
                    "POST /chat/completions returned {status}: {snippet}"
                )));
            }

            let raw: ChatResp = resp
                .json()
                .await
                .map_err(|e| LlmError::Parse(e.to_string()))?;
            let parsed = parse_chat_response(&raw);

            let metadata = CallMetadata {
                model: req.model.model.clone(),
                endpoint: Some(self.endpoint.clone()),
                temperature: req.config.temperature,
                seed,
                cache_hit: false,
            };

            Ok(GenerateOutput {
                response: parsed.response,
                metadata,
                logprobs: parsed.logprobs,
                tool_calls: parsed.tool_calls,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LlmConfig;
    use crate::provider::ToolSpec;
    use pil_core::ModelRef;

    fn base_request(prompt: &str) -> GenerateRequest {
        GenerateRequest::new(
            ModelRef::new("openai", "gpt-oss:20b", None),
            prompt,
            LlmConfig::default(),
            1,
        )
    }

    #[test]
    fn build_request_defaults_have_single_user_message() {
        let req = base_request("hello");
        let body = build_chat_request(&req);
        assert_eq!(body["model"], "gpt-oss:20b");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hello");
        // 既定は決定論（temperature 0.0），seed = base + attempt = 0 + 1 = 1．
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["seed"], 1);
        // 未指定フィールドは省略される．
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("logprobs").is_none());
    }

    #[test]
    fn build_request_prepends_system_message() {
        let mut req = base_request("hi");
        req.config.system = Some("you are a bank agent".to_string());
        req.config.max_tokens = Some(256);
        let body = build_chat_request(&req);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "you are a bank agent");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["max_tokens"], 256);
    }

    #[test]
    fn build_request_maps_tools_and_tool_choice() {
        let req = base_request("do it")
            .with_tools(vec![ToolSpec {
                name: "get_balance".to_string(),
                description: Some("returns balance".to_string()),
                parameters: json!({"type": "object", "properties": {}}),
            }])
            .with_tool_choice("auto".to_string());
        let body = build_chat_request(&req);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "get_balance");
        assert_eq!(
            body["tools"][0]["function"]["description"],
            "returns balance"
        );
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn build_request_sets_logprobs_when_requested() {
        let req = base_request("hi").with_top_logprobs(5);
        let body = build_chat_request(&req);
        assert_eq!(body["logprobs"], true);
        assert_eq!(body["top_logprobs"], 5);
    }

    #[test]
    fn finish_reason_mapping_covers_openai_variants() {
        assert_eq!(map_finish_reason(Some("stop")), FinishReason::Stop);
        assert_eq!(map_finish_reason(None), FinishReason::Stop);
        assert_eq!(map_finish_reason(Some("length")), FinishReason::Length);
        assert_eq!(
            map_finish_reason(Some("content_filter")),
            FinishReason::ContentFilter
        );
        assert_eq!(
            map_finish_reason(Some("tool_calls")),
            FinishReason::ToolCalls
        );
        assert_eq!(
            map_finish_reason(Some("weird")),
            FinishReason::Other("weird".to_string())
        );
    }

    #[test]
    fn parse_plain_text_response() {
        let raw: ChatResp = serde_json::from_str(
            r#"{
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "hi there"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15}
            }"#,
        )
        .unwrap();
        let parsed = parse_chat_response(&raw);
        assert_eq!(parsed.response.text, "hi there");
        assert_eq!(parsed.response.finish_reason, FinishReason::Stop);
        assert_eq!(parsed.response.prompt_tokens, Some(12));
        assert_eq!(parsed.response.completion_tokens, Some(3));
        assert!(parsed.tool_calls.is_empty());
        assert!(parsed.logprobs.is_none());
    }

    #[test]
    fn parse_tool_calls_with_null_content() {
        let raw: ChatResp = serde_json::from_str(
            r#"{
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_abc",
                            "type": "function",
                            "function": {"name": "get_balance", "arguments": "{\"account\":\"x\"}"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            }"#,
        )
        .unwrap();
        let parsed = parse_chat_response(&raw);
        // content が null でもテキストは空文字になる（潰さず素通し）．
        assert_eq!(parsed.response.text, "");
        assert_eq!(parsed.response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call_abc");
        assert_eq!(parsed.tool_calls[0].name, "get_balance");
        assert_eq!(parsed.tool_calls[0].arguments, r#"{"account":"x"}"#);
    }

    #[test]
    fn parse_logprobs_content() {
        let raw: ChatResp = serde_json::from_str(
            r#"{
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "yes"},
                    "finish_reason": "stop",
                    "logprobs": {"content": [{
                        "token": "yes",
                        "logprob": -0.1,
                        "top_logprobs": [
                            {"token": "yes", "logprob": -0.1},
                            {"token": "no", "logprob": -2.3}
                        ]
                    }]}
                }]
            }"#,
        )
        .unwrap();
        let parsed = parse_chat_response(&raw);
        let logprobs = parsed.logprobs.expect("logprobs present");
        assert_eq!(logprobs.len(), 1);
        assert_eq!(logprobs[0].token, "yes");
        assert!((logprobs[0].logprob - (-0.1)).abs() < 1e-9);
        assert_eq!(logprobs[0].top.len(), 2);
        assert_eq!(logprobs[0].top[1].token, "no");
    }

    #[test]
    fn parse_empty_choices_is_safe() {
        let raw: ChatResp = serde_json::from_str(r#"{"choices": []}"#).unwrap();
        let parsed = parse_chat_response(&raw);
        assert_eq!(parsed.response.text, "");
        assert_eq!(parsed.response.finish_reason, FinishReason::Stop);
        assert!(parsed.tool_calls.is_empty());
        assert!(parsed.logprobs.is_none());
    }
}
