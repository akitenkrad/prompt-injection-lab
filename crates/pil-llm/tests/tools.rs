//! M2' ツール呼び出し素通しの network-free テスト（DESIGN §4.1）．
//!
//! `MockProvider` を使い，ツール広告時の決定論的なツール呼び出し・非広告時の従来挙動・
//! `tool_choice="none"` の抑止を確認する．併せて `cache_key` の後方互換（tools=None は
//! バイト不変）と，ツール付き要求が別キーに分かれることを確認する．

use pil_core::{FinishReason, ModelRef};
use pil_llm::{cache_key, GenerateRequest, LlmConfig, LlmProvider, MockProvider, ToolSpec};

fn mock_model() -> ModelRef {
    ModelRef::new("mock", "mock-1", None)
}

fn tool(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: Some(format!("desc of {name}")),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
    }
}

fn base(prompt: &str) -> GenerateRequest {
    GenerateRequest::new(mock_model(), prompt, LlmConfig::default(), 1)
}

#[tokio::test]
async fn mock_returns_tool_call_when_tools_advertised() {
    let provider = MockProvider::new();
    let req = base("do it").with_tools(vec![tool("send_email"), tool("read_file")]);

    let out = provider.generate(&req).await.unwrap();

    assert_eq!(out.response.finish_reason, FinishReason::ToolCalls);
    assert_eq!(out.tool_calls.len(), 1);
    // 先頭ツールを決定論的に呼ぶ
    assert_eq!(out.tool_calls[0].name, "send_email");
    assert_eq!(out.tool_calls[0].arguments, "{}");
    assert!(out.tool_calls[0].id.starts_with("call_"));
    // 決定論: 同一入力なら同一 call_id
    let out2 = provider.generate(&req).await.unwrap();
    assert_eq!(out.tool_calls[0].id, out2.tool_calls[0].id);
}

#[tokio::test]
async fn mock_without_tools_is_unchanged_text_response() {
    let provider = MockProvider::new();
    let out = provider.generate(&base("hello")).await.unwrap();

    assert_eq!(out.response.finish_reason, FinishReason::Stop);
    assert!(out.tool_calls.is_empty());
    assert!(out.response.text.contains("MOCK response"));
}

#[tokio::test]
async fn tool_choice_none_suppresses_tool_call() {
    let provider = MockProvider::new();
    let req = base("do it")
        .with_tools(vec![tool("send_email")])
        .with_tool_choice("none".to_string());

    let out = provider.generate(&req).await.unwrap();
    // tool_choice="none" なら従来のテキスト応答に戻る
    assert_eq!(out.response.finish_reason, FinishReason::Stop);
    assert!(out.tool_calls.is_empty());
    assert!(out.response.text.contains("MOCK response"));
}

#[test]
fn cache_key_is_byte_identical_when_tools_none() {
    // tools=None のキーは M2' 以前とバイト一致（golden 値でリグレッションを固定）．
    // この golden は「tools ブロックを畳み込まない」ことに依存する．
    let key = cache_key(&base("rendered prompt"));
    assert_eq!(
        key,
        "67a2b4a0ea19ae1568cd505d061aedcaaff2ccef029d065d55027b19d310b75b"
    );
}

#[test]
fn cache_key_differs_when_tools_present() {
    let none = cache_key(&base("p"));
    let with_tools = cache_key(&base("p").with_tools(vec![tool("t")]));
    assert_ne!(none, with_tools, "ツール付き要求は別キー");

    // tool_choice が違えば別キー
    let with_choice = cache_key(
        &base("p")
            .with_tools(vec![tool("t")])
            .with_tool_choice("none".to_string()),
    );
    assert_ne!(with_tools, with_choice);
}
