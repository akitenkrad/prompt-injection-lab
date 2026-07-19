//! シム HTTP サーバの loopback 統合テスト（DESIGN §4.1）．**`shim` feature でのみ有効**．
//!
//! エフェメラルポート（`127.0.0.1:0`）で起動し，`MockProvider` を背後に据え，生の HTTP を
//! loopback に流して「外部クライアントの要求が pil-llm 単一経路（MockProvider）に到達すること」・
//! 応答スキーマ・model 反映を検証する．実ネットワークは張らない（loopback のみ）．

#![cfg(feature = "shim")]

use std::net::SocketAddr;
use std::sync::Arc;

use pil_llm::MockProvider;
use pil_shim::server::{serve, ShimState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 生の HTTP/1.1 要求を loopback に送り，`(status, headers＋body 全文)` を返す小さなクライアント．
/// `Connection: close` を指定し，サーバが応答後にソケットを閉じるので `read_to_end` で読み切れる．
async fn http_request(addr: SocketAddr, request: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.expect("connect loopback");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("read response");
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = parse_status(&text);
    (status, text)
}

/// ステータス行 `HTTP/1.1 200 OK` から数値を取り出す．
fn parse_status(response: &str) -> u16 {
    response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("status line")
}

/// ヘッダと本文を分けて本文（JSON）だけを返す．
fn body_of(response: &str) -> &str {
    response.split("\r\n\r\n").nth(1).unwrap_or("")
}

async fn spawn_shim(state: Arc<ShimState>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let _ = serve(listener, state).await;
    });
    addr
}

#[tokio::test]
async fn chat_completion_over_loopback_reaches_mock_provider() {
    let provider = Arc::new(MockProvider::new());
    let state = Arc::new(ShimState::new(
        provider.clone(),
        vec!["mock/mock-1".to_string()],
    ));
    let addr = spawn_shim(state).await;

    let body = r#"{"model":"mock/mock-1","messages":[{"role":"system","content":"be terse"},{"role":"user","content":"hello"}],"temperature":0.7,"seed":42}"#;
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        addr = addr,
        len = body.len(),
        body = body,
    );

    let (status, response) = http_request(addr, &request).await;
    assert_eq!(status, 200, "response was: {response}");

    let json: serde_json::Value =
        serde_json::from_str(body_of(&response)).expect("parse JSON body");
    assert_eq!(json["object"], "chat.completion");
    assert_eq!(json["model"], "mock/mock-1");
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .expect("content string");
    assert!(content.contains("MOCK response"), "content was: {content}");

    // 制御反転の要点: 外部要求が pil-llm 単一経路（MockProvider）に確かに到達した．
    assert_eq!(provider.call_count(), 1);
}

#[tokio::test]
async fn chat_completion_passes_through_logprobs() {
    let provider = Arc::new(MockProvider::new());
    let state = Arc::new(ShimState::new(provider, vec!["mock/mock-1".to_string()]));
    let addr = spawn_shim(state).await;

    let body = r#"{"model":"mock/mock-1","messages":[{"role":"user","content":"hi"}],"logprobs":true,"top_logprobs":3}"#;
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        addr = addr,
        len = body.len(),
        body = body,
    );

    let (status, response) = http_request(addr, &request).await;
    assert_eq!(status, 200, "response was: {response}");

    let json: serde_json::Value =
        serde_json::from_str(body_of(&response)).expect("parse JSON body");
    let top = &json["choices"][0]["logprobs"]["content"][0]["top_logprobs"];
    assert_eq!(top.as_array().expect("top_logprobs array").len(), 3);
}

#[tokio::test]
async fn tool_calling_passthrough_over_loopback() {
    // M2': tools＋tool_choice="auto"＋developer(system) を投げ，tool_calls が返り，
    // MockProvider（単一経路）に到達することを確認する（§4.1）．
    let provider = Arc::new(MockProvider::new());
    let state = Arc::new(ShimState::new(
        provider.clone(),
        vec!["mock/mock-1".to_string()],
    ));
    let addr = spawn_shim(state).await;

    let body = r#"{"model":"mock/mock-1","messages":[{"role":"developer","content":"you are an agent"},{"role":"user","content":"send an email"}],"tools":[{"type":"function","function":{"name":"send_email","description":"send","parameters":{"type":"object","properties":{}}}}],"tool_choice":"auto"}"#;
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        addr = addr,
        len = body.len(),
        body = body,
    );

    let (status, response) = http_request(addr, &request).await;
    assert_eq!(status, 200, "response was: {response}");

    let json: serde_json::Value =
        serde_json::from_str(body_of(&response)).expect("parse JSON body");
    assert_eq!(
        json["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
        "send_email"
    );
    assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
    // content は null（省略）
    assert!(json["choices"][0]["message"]["content"].is_null());
    // 制御反転: 外部要求が pil-llm 単一経路（MockProvider）に到達した．
    assert_eq!(provider.call_count(), 1);
}

#[tokio::test]
async fn multi_turn_tool_result_is_accepted() {
    // M2': 前ターンの assistant tool_call ＋ role="tool" 結果を含む多ターン要求を受理し，
    // 単一経路に funnel されることを確認する（200 で応答）．
    let provider = Arc::new(MockProvider::new());
    let state = Arc::new(ShimState::new(
        provider.clone(),
        vec!["mock/mock-1".to_string()],
    ));
    let addr = spawn_shim(state).await;

    let body = r#"{"model":"mock/mock-1","messages":[{"role":"developer","content":"agent"},{"role":"user","content":"send email"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"send_email","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_1","name":"send_email","content":"sent ok"}],"tools":[{"type":"function","function":{"name":"send_email","parameters":{"type":"object"}}}],"tool_choice":"auto"}"#;
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        addr = addr,
        len = body.len(),
        body = body,
    );

    let (status, response) = http_request(addr, &request).await;
    assert_eq!(status, 200, "response was: {response}");

    let json: serde_json::Value =
        serde_json::from_str(body_of(&response)).expect("parse JSON body");
    assert_eq!(json["object"], "chat.completion");
    // 単一経路（MockProvider）に到達している．
    assert_eq!(provider.call_count(), 1);
}

#[tokio::test]
async fn models_endpoint_lists_configured_models() {
    let provider = Arc::new(MockProvider::new());
    let state = Arc::new(ShimState::new(provider, vec!["mock/mock-1".to_string()]));
    let addr = spawn_shim(state).await;

    let request = format!("GET /v1/models HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (status, response) = http_request(addr, &request).await;
    assert_eq!(status, 200, "response was: {response}");

    let json: serde_json::Value =
        serde_json::from_str(body_of(&response)).expect("parse JSON body");
    assert_eq!(json["object"], "list");
    assert_eq!(json["data"][0]["id"], "mock/mock-1");
    assert_eq!(json["data"][0]["object"], "model");
}
