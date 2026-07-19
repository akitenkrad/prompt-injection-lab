//! OpenAI ⇄ `pil-llm` の純変換（DESIGN §4.1 / §6.2 / §6.3）．
//!
//! ここに置く関数は全て**副作用なし・ネットワーク非依存**であり，`shim` feature 無しで
//! 単体テストできる．HTTP サーバ（[`crate::server`]）はこの純変換を呼ぶだけの薄い殻とし，
//! 「外部（Python）クライアントの生成要求を pil-llm 単一経路に集約する」制御反転（§4.1）を
//! 型の上で成立させる．

use pil_llm::{
    FinishReason, GenerateOutput, GenerateRequest, LlmConfig, LlmError, ModelRef, TokenLogprobs,
    ToolSpec,
};

use crate::openai::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChoiceLogprobs,
    FunctionCall, OpenAiError, OpenAiErrorResponse, TokenLogprobEntry, ToolCallObject,
    TopLogprobEntry, Usage,
};

/// シムが用いる既定の試行番号（§6.2 / §11.4）．
///
/// OpenAI 要求には多試行 ASR の `attempt` 概念が無いため，シム経由の 1 呼び出しは
/// `attempt = 1` として扱う．実送信 seed は `seed_for_attempt(seed, 1)` で導出される（§11.4）．
pub const SHIM_DEFAULT_ATTEMPT: u32 = 1;

/// OpenAI ChatCompletion 要求を [`GenerateRequest`] に写す（§4.1 / §6.2）．
///
/// - `messages` → rendered prompt ＋ `LlmConfig.system`（[`render_messages`]）
/// - `temperature` / `seed` / `max_tokens` → `LlmConfig`（省略時は `LlmConfig` 既定＝決定論）
/// - `logprobs` / `top_logprobs` → `GenerateRequest.top_logprobs`（§6.3）
/// - `model` → [`ModelRef`]（[`model_ref_from_model`]）
pub fn to_generate_request(req: &ChatCompletionRequest) -> GenerateRequest {
    let (prompt, system) = render_messages(&req.messages);

    // 既定値から出発し，指定されたフィールドだけ上書きする（省略時は決定論の既定を保つ）．
    let mut config = LlmConfig::default();
    if let Some(temperature) = req.temperature {
        config.temperature = temperature;
    }
    if let Some(seed) = req.seed {
        config.seed = seed;
    }
    if req.max_tokens.is_some() {
        config.max_tokens = req.max_tokens;
    }
    config.system = system;

    let model = model_ref_from_model(&req.model);
    let mut generate = GenerateRequest::new(model, prompt, config, SHIM_DEFAULT_ATTEMPT);

    // OpenAI 規約では `top_logprobs` は `logprobs=true` 前提だが，§6.3（judge の期待値式）を
    // 取りこぼさないため，どちらか一方が来たら logprobs を要求する（緩め）．
    if req.logprobs == Some(true) || req.top_logprobs.is_some() {
        generate.top_logprobs = Some(req.top_logprobs.unwrap_or(0));
    }

    // ツール情報を単一経路（GenerateRequest）に集約する（§4.1 / M2'）．
    // OpenAI の `tools[].function` を中立表現 `ToolSpec` に写す．
    if let Some(tools) = req.tools.as_ref() {
        let specs: Vec<ToolSpec> = tools
            .iter()
            .map(|tool| ToolSpec {
                name: tool.function.name.clone(),
                description: tool.function.description.clone(),
                parameters: tool.function.parameters.clone(),
            })
            .collect();
        generate.tools = Some(specs);
    }
    // `tool_choice` は文字列（"auto"/"none"/"required"）のみ写す（オブジェクト指定は素通し対象外）．
    if let Some(choice) = req.tool_choice.as_ref().and_then(|v| v.as_str()) {
        generate.tool_choice = Some(choice.to_string());
    }

    generate
}

/// `messages` を rendered prompt と system プロンプトに分解する（§4.1 / §6.2 / M2'）．
///
/// - `role == "system"` **及び `role == "developer"`** は連結して `LlmConfig.system` に載せる
///   （AgentDojo は system を `developer` として送るため同一視する．複数なら改行連結）．
/// - `role == "tool"` は `"tool[<id>|<name>]: <content>"` を 1 行にして畳み込み，
///   多ターンのツール実行結果をプロンプトに含める（決定論的）．
/// - `tool_calls` を伴う assistant は `"assistant: <content?> [tool_call <id> <name>(<args>)…]"`
///   を決定論的に描画し，どのツールをどの引数で呼んだかをプロンプトに残す．
/// - `content` 欠損（None）は空文字として扱う（tool_calls 応答等）．
pub fn render_messages(messages: &[ChatMessage]) -> (String, Option<String>) {
    let mut systems: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    for message in messages {
        let content = message.content.as_deref().unwrap_or("");
        match message.role.as_str() {
            // developer は system と同一視する（AgentDojo 準拠）
            "system" | "developer" => systems.push(content.to_string()),
            "tool" => {
                let id = message.tool_call_id.as_deref().unwrap_or("");
                let name = message.name.as_deref().unwrap_or("");
                lines.push(format!("tool[{id}|{name}]: {content}"));
            }
            role => {
                if message.tool_calls.is_empty() {
                    lines.push(format!("{role}: {content}"));
                } else {
                    // assistant のツール呼び出しを決定論的に描画する．
                    let calls: Vec<String> = message
                        .tool_calls
                        .iter()
                        .map(|call| {
                            format!(
                                "tool_call {} {}({})",
                                call.id, call.function.name, call.function.arguments
                            )
                        })
                        .collect();
                    lines.push(format!("{role}: {content} [{}]", calls.join(", ")));
                }
            }
        }
    }

    let system = if systems.is_empty() {
        None
    } else {
        Some(systems.join("\n"))
    };

    (lines.join("\n"), system)
}

/// OpenAI の `model` 文字列を [`ModelRef`] に写す（§5.5）．
///
/// `"provider/model"` 形式なら provider を割り出し，そうでなければ provider を `"openai"` とする
/// （OpenAI 互換要求のため）．endpoint は呼び出し側（プロバイダ）が持つため常に None．
pub fn model_ref_from_model(model: &str) -> ModelRef {
    match model.split_once('/') {
        Some((provider, name)) if !provider.is_empty() && !name.is_empty() => {
            ModelRef::new(provider, name, None)
        }
        _ => ModelRef::new("openai", model, None),
    }
}

/// [`GenerateOutput`] を OpenAI ChatCompletion 応答に写す（§4.1 / §6.3）．
///
/// `model` には要求で指定されたモデル名をそのまま反映する（OpenAI 互換）．`id` は応答本文から
/// 決定論的に導出し，`created` は 0 に固定する（純変換の決定性を保つ）．
pub fn to_chat_completion_response(output: &GenerateOutput, model: &str) -> ChatCompletionResponse {
    let response = &output.response;

    let logprobs = output.logprobs.as_ref().map(|tokens| map_logprobs(tokens));

    // ツール呼び出しがあれば OpenAI 形 `tool_calls[]` に写し，content は null（None）にする（M2'）．
    // 無ければ従来通りテキスト応答（content = 本文）を返す．
    let message = if output.tool_calls.is_empty() {
        ChatMessage {
            role: "assistant".to_string(),
            content: Some(response.text.clone()),
            ..Default::default()
        }
    } else {
        let tool_calls = output
            .tool_calls
            .iter()
            .map(|call| ToolCallObject {
                id: call.id.clone(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            })
            .collect();
        ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls,
            ..Default::default()
        }
    };

    let choice = ChatChoice {
        index: 0,
        message,
        finish_reason: Some(finish_reason_to_openai(&response.finish_reason)),
        logprobs,
    };

    let usage = match (response.prompt_tokens, response.completion_tokens) {
        (Some(prompt_tokens), Some(completion_tokens)) => Some(Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
        }),
        _ => None,
    };

    ChatCompletionResponse {
        id: response_id(&response.text),
        object: "chat.completion".to_string(),
        created: 0,
        model: model.to_string(),
        choices: vec![choice],
        usage,
    }
}

/// `pil-llm` の logprobs を OpenAI `choices[].logprobs` 形に写す（§6.3）．
fn map_logprobs(tokens: &[TokenLogprobs]) -> ChoiceLogprobs {
    let content = tokens
        .iter()
        .map(|token| TokenLogprobEntry {
            token: token.token.clone(),
            logprob: token.logprob,
            bytes: None,
            top_logprobs: token
                .top
                .iter()
                .map(|candidate| TopLogprobEntry {
                    token: candidate.token.clone(),
                    logprob: candidate.logprob,
                    bytes: None,
                })
                .collect(),
        })
        .collect();

    ChoiceLogprobs { content }
}

/// [`FinishReason`] を OpenAI の `finish_reason` 文字列に写す．
pub fn finish_reason_to_openai(finish_reason: &FinishReason) -> String {
    match finish_reason {
        FinishReason::Stop => "stop".to_string(),
        FinishReason::Length => "length".to_string(),
        FinishReason::ContentFilter => "content_filter".to_string(),
        FinishReason::ToolCalls => "tool_calls".to_string(),
        FinishReason::Other(raw) => raw.clone(),
    }
}

/// [`LlmError`] を OpenAI 形式のエラー応答と HTTP ステータスに写す（§6）．
///
/// 返り値の `u16` はステータスコードで，サーバ側が `StatusCode` に変換する（純変換を http 型から切り離す）．
pub fn error_to_openai(error: &LlmError) -> (u16, OpenAiErrorResponse) {
    let (status, error_type) = match error {
        LlmError::NotImplemented(_) => (501, "not_implemented"),
        LlmError::UnsupportedVersion { .. } => (500, "unsupported_version"),
        LlmError::UnparsableVersion(_) => (500, "unparsable_version"),
        LlmError::Network(_) => (502, "network_error"),
        LlmError::Provider(_) => (502, "upstream_error"),
        LlmError::Parse(_) => (502, "parse_error"),
    };

    let body = OpenAiErrorResponse {
        error: OpenAiError {
            message: error.to_string(),
            error_type: error_type.to_string(),
            code: None,
            param: None,
        },
    };

    (status, body)
}

/// 応答本文から決定論的に `chatcmpl-...` の id を導出する（blake3 先頭 16 hex 桁）．
fn response_id(text: &str) -> String {
    let hex = blake3::hash(text.as_bytes()).to_hex();
    format!("chatcmpl-{}", &hex[..16])
}
