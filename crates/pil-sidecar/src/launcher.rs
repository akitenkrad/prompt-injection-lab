//! Python sidecar プロセスの起動・ライフサイクル制御（DESIGN §4.1）．**`sidecar` feature でのみ有効**．
//!
//! [`run_sidecar`] は [`SidecarConfig`] に従って `tokio::process::Command` でプロセスを起動し，
//! シムへ向ける環境変数を注入し，タイムアウト付きで終了を待ち，stdout/stderr を捕捉して
//! [`SidecarRun`] に正規化する．プロセスの kill は `kill_on_drop` によりタイムアウト時／future の
//! drop 時に確実に行われる．この配線は「測定値を変えないグルー」であり，Rust に集約する（§4.1）．

use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::config::{SidecarConfig, SidecarRun};

/// sidecar 起動・待機の失敗（DESIGN §4.1）．
#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    /// プロセスの起動自体に失敗した（プログラム不在等）．
    #[error("sidecar プロセスの起動に失敗しました: {0}")]
    Spawn(#[source] std::io::Error),
    /// 制限時間内に終了しなかった（プロセスは kill 済み）．
    #[error("sidecar の実行が {0:?} を超えてタイムアウトしました")]
    Timeout(Duration),
    /// 実行中の入出力取得に失敗した．
    #[error("sidecar の入出力取得に失敗しました: {0}")]
    Io(#[source] std::io::Error),
}

/// `config` に従って Python sidecar を起動し，`timeout` 以内の終了を待って provenance を返す（§4.1）．
///
/// - `OPENAI_BASE_URL` / `OPENAI_API_KEY` を注入し（[`SidecarConfig::injected_env`]），Python の
///   OpenAI 互換クライアントを pil-llm 単一経路（シム）へ向ける．
/// - stdin は閉じ，stdout/stderr は pipe で捕捉する．
/// - `kill_on_drop(true)` により，タイムアウトで future を捨てた時点で子プロセスは kill される．
pub async fn run_sidecar(
    config: &SidecarConfig,
    timeout: Duration,
) -> Result<SidecarRun, SidecarError> {
    let mut cmd = Command::new(&config.program);
    cmd.arg(&config.script);
    cmd.args(&config.args);
    if let Some(dir) = &config.working_dir {
        cmd.current_dir(dir);
    }
    for (key, value) in config.injected_env() {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let child = cmd.spawn().map_err(SidecarError::Spawn)?;

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(SidecarError::Io(error)),
        // future を捨てると子プロセスは kill_on_drop で kill される（§4.1）．
        Err(_elapsed) => return Err(SidecarError::Timeout(timeout)),
    };

    Ok(SidecarRun::new(
        config,
        output.status.success(),
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}
