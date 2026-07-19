//! sidecar → シム → pil-llm 単一経路の funnel を end-to-end で証明する統合テスト（DESIGN §4.1）．
//! **`sidecar` feature でのみ有効**．実ネットワークは張らず loopback のみを用いる（§6.1）．
//!
//! 経路: 極小 Python スタブ（stdlib のみ）→ 注入された `OPENAI_BASE_URL` のシム → `MockProvider`．
//! `MockProvider::call_count() == 1` が「Python の呼び出しが第 2 のモデル経路を開かず，pil-llm の
//! 単一経路へ確かに funnel された」ことの証拠になる（制御の反転の要点）．

#![cfg(feature = "sidecar")]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pil_llm::MockProvider;
use pil_shim::server::{serve, ShimState};
use pil_sidecar::{run_sidecar, SidecarConfig};
use tokio::net::TcpListener;

/// エフェメラルポートでシムを起動し，その loopback アドレスを返す（pil-shim の `serve` を用いる）．
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

/// fixtures/sidecar_stub.py の絶対パス．
fn stub_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sidecar_stub.py")
}

/// python3 が使えない環境ではスキップする（存在すれば既定で走らせる）．
fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn sidecar_call_is_funneled_through_pil_llm_single_path() {
    if !python3_available() {
        eprintln!("python3 が見つからないため funnel テストをスキップします");
        return;
    }

    // 1. MockProvider を背後に据えたシムをエフェメラルポートで起動する．
    let provider = Arc::new(MockProvider::new());
    let state = Arc::new(ShimState::new(
        provider.clone(),
        vec!["mock/mock-1".to_string()],
    ));
    let addr = spawn_shim(state).await;

    // 2. シムの base_url を注入して Python スタブを launcher で起動する．
    let base_url = format!("http://{addr}/v1");
    let config = SidecarConfig::new("python3", stub_script(), base_url.clone());
    let run = run_sidecar(&config, Duration::from_secs(30))
        .await
        .expect("sidecar run");

    // 3. プロセスは正常終了し，stdout にモック応答本文が乗っている．
    assert!(
        run.success,
        "sidecar failed: stdout={:?} stderr={:?}",
        run.stdout, run.stderr
    );
    assert_eq!(run.exit_code, Some(0));
    assert!(
        run.stdout.contains("MOCK response"),
        "stdout was: {:?}",
        run.stdout
    );
    assert_eq!(run.base_url, base_url);

    // 4. funnel の要点: Python の呼び出しは pil-llm 単一経路（MockProvider）にちょうど 1 回だけ到達した．
    assert_eq!(provider.call_count(), 1);
}
