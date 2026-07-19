//! OpenAI ⇄ `pil-llm` の純変換（DESIGN §4.1 / §6.2 / §6.3）．
//!
//! ここに置く関数は全て**副作用なし・ネットワーク非依存**であり，`shim` feature 無しで
//! 単体テストできる．HTTP サーバ（[`crate::server`]）はこの純変換を呼ぶだけの薄い殻とし，
//! 「外部（Python）クライアントの生成要求を pil-llm 単一経路に集約する」制御反転（§4.1）を
//! 型の上で成立させる．

use pil_llm::{
    FinishReason, GenerateOutput, GenerateRequest, LlmConfig, LlmError, ModelRef, TokenLogprobs,
};

use crate::openai::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChoiceLogprobs,
    OpenAiError, OpenAiErrorResponse, TokenLogprobEntry, TopLogprobEntry, Usage,
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

    generate
}

/// `messages` を rendered prompt と system プロンプトに分解する（§4.1 / §6.2）．
///
/// - `role == "system"` のメッセージは連結して `LlmConfig.system` に載せる（複数なら改行連結）．
/// - それ以外は `"{role}: {content}"` を改行で連結し，最終送信プロンプトにする（決定論的順序）．
pub fn render_messages(messages: &[ChatMessage]) -> (String, Option<String>) {
    let mut systems: Vec<&str> = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    for message in messages {
        if message.role == "system" {
            systems.push(message.content.as_str());
        } else {
            lines.push(format!("{}: {}", message.role, message.content));
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

    let choice = ChatChoice {
        index: 0,
        message: ChatMessage {
            role: "assistant".to_string(),
            content: response.text.clone(),
        },
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
