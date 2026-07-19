//! 純変換のテスト（DESIGN §4.1 / §6.3）．
//!
//! 既定 feature（network-free）で走る．OpenAI ⇄ pil-llm の写像を項目ごとに検証する．

use pil_llm::{
    CallMetadata, FinishReason, GenerateOutput, LlmError, ModelRef, Response, TokenLogprobs,
    TopLogprob,
};
use pil_shim::mapping::{
    error_to_openai, finish_reason_to_openai, model_ref_from_model, to_chat_completion_response,
    to_generate_request, SHIM_DEFAULT_ATTEMPT,
};
use pil_shim::openai::{ChatCompletionRequest, ChatMessage};

fn message(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: role.to_string(),
        content: content.to_string(),
    }
}

#[test]
fn request_maps_to_generate_request_fieldwise() {
    let request = ChatCompletionRequest {
        model: "acme/model-x".to_string(),
        messages: vec![
            message("system", "be terse"),
            message("user", "hello"),
            message("assistant", "hi"),
            message("user", "bye"),
        ],
        temperature: Some(0.7),
        seed: Some(42),
        max_tokens: Some(128),
        logprobs: Some(true),
        top_logprobs: Some(5),
        stream: None,
    };

    let generate = to_generate_request(&request);

    assert_eq!(generate.model, ModelRef::new("acme", "model-x", None));
    assert_eq!(generate.config.system.as_deref(), Some("be terse"));
    assert_eq!(generate.prompt, "user: hello\nassistant: hi\nuser: bye");
    assert!((generate.config.temperature - 0.7).abs() < 1e-6);
    assert_eq!(generate.config.seed, 42);
    assert_eq!(generate.config.max_tokens, Some(128));
    assert_eq!(generate.attempt, SHIM_DEFAULT_ATTEMPT);
    assert_eq!(generate.top_logprobs, Some(5));
}

#[test]
fn multiple_system_messages_are_joined() {
    let request = ChatCompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![
            message("system", "line1"),
            message("system", "line2"),
            message("user", "go"),
        ],
        temperature: None,
        seed: None,
        max_tokens: None,
        logprobs: None,
        top_logprobs: None,
        stream: None,
    };

    let generate = to_generate_request(&request);
    assert_eq!(generate.config.system.as_deref(), Some("line1\nline2"));
    assert_eq!(generate.prompt, "user: go");
}

#[test]
fn omitted_fields_use_deterministic_defaults() {
    let request = ChatCompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![message("user", "hi")],
        temperature: None,
        seed: None,
        max_tokens: None,
        logprobs: None,
        top_logprobs: None,
        stream: None,
    };

    let generate = to_generate_request(&request);
    assert!((generate.config.temperature - 0.0).abs() < 1e-6);
    assert_eq!(generate.config.seed, 0);
    assert_eq!(generate.config.max_tokens, None);
    assert_eq!(generate.config.system, None);
    assert_eq!(generate.top_logprobs, None);
}

#[test]
fn top_logprobs_without_logprobs_flag_still_requested() {
    let request = ChatCompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![message("user", "hi")],
        temperature: None,
        seed: None,
        max_tokens: None,
        logprobs: None,
        top_logprobs: Some(3),
        stream: None,
    };

    let generate = to_generate_request(&request);
    assert_eq!(generate.top_logprobs, Some(3));
}

#[test]
fn output_maps_to_chat_completion_response() {
    let output = GenerateOutput {
        response: Response {
            text: "hello world".to_string(),
            finish_reason: FinishReason::Length,
            prompt_tokens: Some(3),
            completion_tokens: Some(2),
            reached_clip_limit: false,
        },
        metadata: CallMetadata {
            model: "acme/model-x".to_string(),
            endpoint: None,
            temperature: 0.0,
            seed: 1,
            cache_hit: false,
        },
        logprobs: None,
    };

    let response = to_chat_completion_response(&output, "acme/model-x");

    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.model, "acme/model-x");
    assert!(response.id.starts_with("chatcmpl-"));
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(response.choices[0].message.content, "hello world");
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("length"));
    assert!(response.choices[0].logprobs.is_none());

    let usage = response.usage.expect("usage present when tokens returned");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 5);
}

#[test]
fn logprobs_pass_through_to_openai_shape() {
    let output = GenerateOutput {
        response: Response {
            text: "t0".to_string(),
            finish_reason: FinishReason::Stop,
            prompt_tokens: None,
            completion_tokens: None,
            reached_clip_limit: false,
        },
        metadata: CallMetadata {
            model: "m".to_string(),
            endpoint: None,
            temperature: 0.0,
            seed: 0,
            cache_hit: false,
        },
        logprobs: Some(vec![TokenLogprobs {
            token: "t0".to_string(),
            logprob: -0.1,
            top: vec![
                TopLogprob {
                    token: "t0".to_string(),
                    logprob: -0.1,
                },
                TopLogprob {
                    token: "t1".to_string(),
                    logprob: -1.1,
                },
            ],
        }]),
    };

    let response = to_chat_completion_response(&output, "m");

    let logprobs = response.choices[0]
        .logprobs
        .as_ref()
        .expect("logprobs present when provider returns them");
    assert_eq!(logprobs.content.len(), 1);
    assert_eq!(logprobs.content[0].token, "t0");
    assert_eq!(logprobs.content[0].top_logprobs.len(), 2);
    assert_eq!(logprobs.content[0].top_logprobs[1].token, "t1");
    // usage は None（トークン数なし）
    assert!(response.usage.is_none());
}

#[test]
fn finish_reason_conversions() {
    assert_eq!(finish_reason_to_openai(&FinishReason::Stop), "stop");
    assert_eq!(finish_reason_to_openai(&FinishReason::Length), "length");
    assert_eq!(
        finish_reason_to_openai(&FinishReason::ContentFilter),
        "content_filter"
    );
    assert_eq!(
        finish_reason_to_openai(&FinishReason::ToolCalls),
        "tool_calls"
    );
    assert_eq!(
        finish_reason_to_openai(&FinishReason::Other("weird".to_string())),
        "weird"
    );
}

#[test]
fn model_ref_conversions() {
    assert_eq!(
        model_ref_from_model("ollama/llama3:8b"),
        ModelRef::new("ollama", "llama3:8b", None)
    );
    assert_eq!(
        model_ref_from_model("gpt-4"),
        ModelRef::new("openai", "gpt-4", None)
    );
}

#[test]
fn errors_map_to_openai_status_and_type() {
    let (status, body) = error_to_openai(&LlmError::NotImplemented("openai".to_string()));
    assert_eq!(status, 501);
    assert_eq!(body.error.error_type, "not_implemented");
    assert!(body.error.message.contains("not implemented"));

    let (status, body) = error_to_openai(&LlmError::Network("down".to_string()));
    assert_eq!(status, 502);
    assert_eq!(body.error.error_type, "network_error");

    let (status, body) = error_to_openai(&LlmError::Provider("rate limit".to_string()));
    assert_eq!(status, 502);
    assert_eq!(body.error.error_type, "upstream_error");
}
