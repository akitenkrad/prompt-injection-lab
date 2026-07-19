//! 純変換のテスト（DESIGN §4.1 / §6.3）．
//!
//! 既定 feature（network-free）で走る．OpenAI ⇄ pil-llm の写像を項目ごとに検証する．

use pil_llm::{
    CallMetadata, FinishReason, GenerateOutput, LlmError, ModelRef, Response, TokenLogprobs,
    ToolCall, TopLogprob,
};
use pil_shim::mapping::{
    error_to_openai, finish_reason_to_openai, model_ref_from_model, render_messages,
    to_chat_completion_response, to_generate_request, SHIM_DEFAULT_ATTEMPT,
};
use pil_shim::openai::{
    ChatCompletionRequest, ChatCompletionTool, ChatMessage, FunctionCall, FunctionDef,
    ToolCallObject,
};

fn message(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: role.to_string(),
        content: Some(content.to_string()),
        ..Default::default()
    }
}

fn base_request(model: &str, messages: Vec<ChatMessage>) -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: model.to_string(),
        messages,
        temperature: None,
        seed: None,
        max_tokens: None,
        logprobs: None,
        top_logprobs: None,
        stream: None,
        tools: None,
        tool_choice: None,
    }
}

fn function_tool(name: &str) -> ChatCompletionTool {
    ChatCompletionTool {
        tool_type: "function".to_string(),
        function: FunctionDef {
            name: name.to_string(),
            description: Some(format!("desc of {name}")),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        },
    }
}

fn text_output(text: &str, finish_reason: FinishReason) -> GenerateOutput {
    GenerateOutput {
        response: Response {
            text: text.to_string(),
            finish_reason,
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
        logprobs: None,
        tool_calls: Vec::new(),
    }
}

#[test]
fn request_maps_to_generate_request_fieldwise() {
    let request = ChatCompletionRequest {
        temperature: Some(0.7),
        seed: Some(42),
        max_tokens: Some(128),
        logprobs: Some(true),
        top_logprobs: Some(5),
        ..base_request(
            "acme/model-x",
            vec![
                message("system", "be terse"),
                message("user", "hello"),
                message("assistant", "hi"),
                message("user", "bye"),
            ],
        )
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
    let request = base_request(
        "gpt-4",
        vec![
            message("system", "line1"),
            message("system", "line2"),
            message("user", "go"),
        ],
    );

    let generate = to_generate_request(&request);
    assert_eq!(generate.config.system.as_deref(), Some("line1\nline2"));
    assert_eq!(generate.prompt, "user: go");
}

#[test]
fn omitted_fields_use_deterministic_defaults() {
    let request = base_request("gpt-4", vec![message("user", "hi")]);

    let generate = to_generate_request(&request);
    assert!((generate.config.temperature - 0.0).abs() < 1e-6);
    assert_eq!(generate.config.seed, 0);
    assert_eq!(generate.config.max_tokens, None);
    assert_eq!(generate.config.system, None);
    assert_eq!(generate.top_logprobs, None);
    assert_eq!(generate.tools, None);
    assert_eq!(generate.tool_choice, None);
}

#[test]
fn top_logprobs_without_logprobs_flag_still_requested() {
    let request = ChatCompletionRequest {
        top_logprobs: Some(3),
        ..base_request("gpt-4", vec![message("user", "hi")])
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
        tool_calls: Vec::new(),
    };

    let response = to_chat_completion_response(&output, "acme/model-x");

    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.model, "acme/model-x");
    assert!(response.id.starts_with("chatcmpl-"));
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("hello world")
    );
    assert!(response.choices[0].message.tool_calls.is_empty());
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
        tool_calls: Vec::new(),
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

// ---- M2' ツール呼び出し素通し（§4.1）----

#[test]
fn tools_and_tool_choice_map_to_generate_request() {
    // OpenAI の tools（function）と tool_choice="auto" が単一経路（GenerateRequest）に集約される
    let request = ChatCompletionRequest {
        tools: Some(vec![
            function_tool("send_email"),
            function_tool("read_file"),
        ]),
        tool_choice: Some(serde_json::json!("auto")),
        ..base_request("acme/model-x", vec![message("user", "do it")])
    };

    let generate = to_generate_request(&request);
    let tools = generate.tools.expect("tools populated");
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "send_email");
    assert_eq!(tools[0].description.as_deref(), Some("desc of send_email"));
    assert_eq!(tools[0].parameters["type"], "object");
    assert_eq!(tools[1].name, "read_file");
    assert_eq!(generate.tool_choice.as_deref(), Some("auto"));
}

#[test]
fn developer_role_is_treated_as_system() {
    // AgentDojo は system を role="developer" として送る → system に畳み込む
    let request = base_request(
        "gpt-4",
        vec![
            message("developer", "you are an agent"),
            message("user", "hi"),
        ],
    );
    let generate = to_generate_request(&request);
    assert_eq!(generate.config.system.as_deref(), Some("you are an agent"));
    assert_eq!(generate.prompt, "user: hi");
}

#[test]
fn tool_result_message_is_rendered_into_prompt() {
    // role="tool"（tool_call_id/name＋content）が決定論的にプロンプトへ入る
    let tool_msg = ChatMessage {
        role: "tool".to_string(),
        content: Some("42 results".to_string()),
        tool_call_id: Some("call_abc".to_string()),
        name: Some("search".to_string()),
        ..Default::default()
    };
    let (prompt, _system) = render_messages(&[message("user", "search"), tool_msg]);
    assert_eq!(prompt, "user: search\ntool[call_abc|search]: 42 results");
}

#[test]
fn assistant_tool_calls_are_rendered_into_prompt() {
    // 前ターンの assistant tool_calls も決定論的に描画される（多ターン駆動）
    let assistant = ChatMessage {
        role: "assistant".to_string(),
        content: None,
        tool_calls: vec![ToolCallObject {
            id: "call_1".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "send_email".to_string(),
                arguments: "{\"to\":\"x\"}".to_string(),
            },
        }],
        ..Default::default()
    };
    let (prompt, _system) = render_messages(&[assistant]);
    assert_eq!(
        prompt,
        "assistant:  [tool_call call_1 send_email({\"to\":\"x\"})]"
    );
}

#[test]
fn output_with_tool_calls_maps_to_openai_tool_calls() {
    // GenerateOutput.tool_calls → choices[0].message.tool_calls，content は null，finish=tool_calls
    let mut output = text_output("", FinishReason::ToolCalls);
    output.tool_calls = vec![ToolCall {
        id: "call_xyz".to_string(),
        name: "send_email".to_string(),
        arguments: "{}".to_string(),
    }];

    let response = to_chat_completion_response(&output, "acme/model-x");
    let msg = &response.choices[0].message;
    assert!(
        msg.content.is_none(),
        "content should be null on tool_calls"
    );
    assert_eq!(msg.tool_calls.len(), 1);
    assert_eq!(msg.tool_calls[0].id, "call_xyz");
    assert_eq!(msg.tool_calls[0].call_type, "function");
    assert_eq!(msg.tool_calls[0].function.name, "send_email");
    assert_eq!(msg.tool_calls[0].function.arguments, "{}");
    assert_eq!(
        response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );

    // JSON でも OpenAI 形（content 省略 = null 相当，tool_calls 配列）になる
    let value = serde_json::to_value(&response).unwrap();
    assert!(value["choices"][0]["message"].get("content").is_none());
    assert_eq!(
        value["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
        "send_email"
    );
}
