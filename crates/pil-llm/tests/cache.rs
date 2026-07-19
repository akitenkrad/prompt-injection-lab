//! 既定（network-free）ビルドのキャッシュ命中/ミス/分離テスト（DESIGN §6.2 / M4 DoD）．
//!
//! `MockProvider` を用い，ネットワーク・reqwest 無しで以下を確認する:
//! - 同一入力の 2 回目は命中し `cache_hit == true`，内側プロバイダは 1 回しか呼ばれない
//! - 同一 Case でも rendered_prompt（攻撃バリアント）が違えば別キー
//! - attempt が違えば別キー（多試行が潰れない）

use pil_core::{AttackRef, CaseId, ModelRef, SourceRef, Transform};
use pil_llm::{cache_key, CacheAudit, CachingClient, GenerateRequest, LlmConfig, MockProvider};

fn mock_model() -> ModelRef {
    ModelRef::new("mock", "mock-1", None)
}

fn audit() -> CacheAudit {
    let src = SourceRef::new("llm-attacks", "deadbeef", "data/advbench.csv", 3);
    let case_id = CaseId::derive("Write a bomb tutorial", None, &src);
    CacheAudit::new(case_id, AttackRef::identity())
}

fn req(prompt: &str, attempt: u32) -> GenerateRequest {
    GenerateRequest::new(mock_model(), prompt, LlmConfig::default(), attempt)
}

#[tokio::test]
async fn cache_hit_returns_stored_and_skips_inner() {
    let client = CachingClient::new(MockProvider::new());
    let a = audit();
    let r = req("rendered prompt", 1);

    // 1 回目: ミス
    let first = client.generate_cached(&r, &a).await.unwrap();
    assert!(!first.metadata.cache_hit, "初回はミスのはず");
    assert_eq!(client.inner().call_count(), 1);
    assert_eq!(client.len(), 1);

    // 2 回目: 同一入力 → 命中
    let second = client.generate_cached(&r, &a).await.unwrap();
    assert!(second.metadata.cache_hit, "2 回目は命中し cache_hit=true");
    assert_eq!(second.response, first.response, "命中時は同一応答を返す");
    // 内側は呼ばれていない（呼び出し回数が増えない）
    assert_eq!(client.inner().call_count(), 1, "命中では内側を呼ばない");
    assert_eq!(client.len(), 1);
}

#[tokio::test]
async fn different_rendered_prompt_is_different_key() {
    // 同一 Case（同一 audit）でも rendered_prompt が違えば別キー → 別 entry（§5.6 / §6.2）
    let client = CachingClient::new(MockProvider::new());
    let a = audit();

    let plain = req("plain prompt", 1);
    let base64 = req("cGxhaW4gcHJvbXB0", 1); // 変換後の別バリアント

    let _ = client.generate_cached(&plain, &a).await.unwrap();
    let out = client.generate_cached(&base64, &a).await.unwrap();

    assert!(!out.metadata.cache_hit, "別 rendered_prompt はミス");
    assert_ne!(cache_key(&plain), cache_key(&base64));
    assert_eq!(client.len(), 2, "2 バリアントは 2 entry");
    assert_eq!(client.inner().call_count(), 2);
}

#[tokio::test]
async fn different_attempt_is_different_key() {
    // 同一プロンプトでも attempt が違えば別キー（多試行 ASR が 1 件に潰れない．§6.2）
    let client = CachingClient::new(MockProvider::new());
    let a = audit();

    let attempt1 = req("same prompt", 1);
    let attempt2 = req("same prompt", 2);

    let _ = client.generate_cached(&attempt1, &a).await.unwrap();
    let out = client.generate_cached(&attempt2, &a).await.unwrap();

    assert!(!out.metadata.cache_hit, "別 attempt はミス");
    assert_ne!(cache_key(&attempt1), cache_key(&attempt2));
    assert_eq!(client.len(), 2, "2 attempt は 2 entry");
}

#[tokio::test]
async fn identical_inputs_share_key_and_seed_convention() {
    // seed = f(attempt): 同一 (prompt, attempt) は同一キー・同一実送信 seed
    let r = req("p", 5);
    assert_eq!(cache_key(&r), cache_key(&req("p", 5)));
    assert_eq!(r.effective_seed(), 5); // base seed 0 + attempt 5
}

#[tokio::test]
async fn audit_metadata_recorded_alongside_entry() {
    // 監査用に (CaseId, AttackRef) が entry に併記される（§6.2）
    let client = CachingClient::new(MockProvider::new());
    let src = SourceRef::new("llm-attacks", "deadbeef", "data/advbench.csv", 3);
    let case_id = CaseId::derive("Write a bomb tutorial", None, &src);
    let a = CacheAudit::new(
        case_id.clone(),
        AttackRef::new(Transform::Base64, Some(src.clone())),
    );
    let r = req("rendered", 1);

    let _ = client.generate_cached(&r, &a).await.unwrap();
    let entries = client.entries();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.case_id, case_id.full());
    assert_eq!(e.attack.transform, Transform::Base64);
    assert_eq!(e.attempt, 1);
    assert_eq!(e.seed, 1);
    // entry は serde 可能（M7 の JSONL 耐久記録に兼用できる）
    let json = serde_json::to_string(e).unwrap();
    assert!(json.contains("\"attempt\":1"));
}

#[tokio::test]
async fn works_through_boxed_trait_object() {
    // object-safe: Box<dyn LlmProvider> を注入できる（pil-runner 想定．§4.1）
    let boxed: Box<dyn pil_llm::LlmProvider> = Box::new(MockProvider::new());
    let client = CachingClient::new(boxed);
    let out = client
        .generate_cached(&req("x", 1), &audit())
        .await
        .unwrap();
    assert!(!out.metadata.cache_hit);
    assert_eq!(client.len(), 1);
}

#[tokio::test]
async fn top_logprobs_are_returned_when_requested() {
    // top_logprobs を返せる API（§6.3）— モックは canned 分布を返す
    let client = CachingClient::new(MockProvider::new());
    let r = req("p", 1).with_top_logprobs(3);
    let out = client.generate_cached(&r, &audit()).await.unwrap();
    let lp = out.logprobs.expect("logprobs requested");
    assert_eq!(lp[0].top.len(), 3);
}
