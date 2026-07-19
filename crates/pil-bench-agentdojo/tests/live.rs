//! sidecar 駆動の AgentDojo ライブ実行の配線（DESIGN §4.1）．**`agentdojo` feature でのみコンパイル**．
//!
//! この経路は network-free ではない．実行には以下が全て要る:
//!   1. **agentdojo の pip インストール**（多数の重い deps）．
//!   2. **シムの OpenAI tool-calling 対応**（`tools`/`tool_calls`/`developer` role）= M2'．P2-M1 の
//!      `MockProvider` は tool_calls を返せないため，このテストは配線のみを検証し `#[ignore]`．
//!   3. **実ツール対応の LLM**．
//!
//! したがって本テストは既定 CI から外す（`#[ignore]`）．目的は「shim を立て，`OPENAI_BASE_URL` を
//! 注入して `python/run_case.py` を sidecar 起動する」配線が **feature 有効時にコンパイル・実行可能**で
//! あることの担保である（測定値を変えないグルーの検証，§4.1）．重い依存（tokio/axum）は
//! dev-dependencies に置くため，既定の `cargo tree -e no-dev` には現れない（§6.1）．
#![cfg(feature = "agentdojo")]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pil_bench_agentdojo::{parse_result, Verdict};
use pil_llm::MockProvider;
use pil_shim::server::{serve, ShimState};
use pil_sidecar::{run_sidecar, SidecarConfig};
use tokio::net::TcpListener;

/// エフェメラルポートでシムを起動し，loopback アドレスを返す（pil-shim の `serve`）．
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

/// `python/run_case.py` の絶対パス．
fn run_case_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("python")
        .join("run_case.py")
}

/// ライブ 1 ケース実行の配線（agentdojo 実インストール + シム tool-calling + 実モデルが前提）．
///
/// `#[ignore]`: 既定 CI では走らせない（上記 3 前提が揃わないため）．feature 有効時に**コンパイル**し，
/// shim 起動 → env 注入 → sidecar 起動 → 結果 JSON パースまでの型が通ることを担保する．
#[tokio::test]
#[ignore = "agentdojo 実インストール + シム tool-calling(M2') + 実ツール対応モデルが必要（M4b）"]
async fn agentdojo_live_case_wiring() {
    // 1. シムをエフェメラルポートで起動する（実運用では tool-calling 対応 provider に差し替える）．
    let provider = Arc::new(MockProvider::new());
    let state = Arc::new(ShimState::new(
        provider.clone(),
        vec!["gpt-4o-2024-05-13".to_string()],
    ));
    let addr = spawn_shim(state).await;

    // 2. シムの base_url を注入して run_case.py を sidecar 起動する（モデル呼び出しは env でシムへ）．
    let base_url = format!("http://{addr}/v1");
    let config = SidecarConfig::new("python3", run_case_script(), base_url.clone()).with_args([
        "--suite",
        "banking",
        "--user-task",
        "user_task_0",
        "--injection-task",
        "injection_task_1",
        "--attack",
        "important_instructions",
        "--model",
        "gpt-4o-2024-05-13",
    ]);

    let run = run_sidecar(&config, Duration::from_secs(600))
        .await
        .expect("sidecar run");
    assert_eq!(run.base_url, base_url);

    // 3. driver の {utility, security, error} を注入次元の Verdict に正規化する．
    let result = parse_result(&run.stdout).expect("parse run_case.py output");
    assert!(matches!(
        result.verdict(),
        Verdict::Success | Verdict::Failure | Verdict::Undecidable { .. }
    ));
}
