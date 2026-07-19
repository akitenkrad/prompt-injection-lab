//! `pil-sidecar` — Python sidecar 起動基盤（DESIGN §4.1「制御の反転」/ native-first）．
//!
//! §4.1 の要求は「sidecar を素直に作るとモデル呼び出しが 2 系統に分裂する」ことの回避である．
//! そこで Rust プロセスが [`pil-shim`](../pil_shim/index.html) の OpenAI 互換エンドポイントを立て，
//! Python 側の `base_url` をそこへ向けさせる．本 crate はその配線のうち **プロセス起動・環境変数注入・
//! 入出力正規化・provenance 記録**という「測定値を変えないグルー」を Rust に集約する（§4.1）．
//! Python 側は irreducible な環境／ツール本体（M3 ではスタブ）だけを持つ薄い殻に留める（native-first）．
//!
//! - **adapter 種別 = `sidecar`** は実装都合であり，`EnvKind`（科学的性質）とは別物である（§4.2）．
//! - **既定ビルドは network-free**（§6.1）: [`SidecarConfig`] / [`SidecarRun`] の純データ型のみを含む．
//!   実際にプロセスを起動する [`run_sidecar`] は feature `sidecar`（`tokio::process`）でのみ導入する．
//!   既定では tokio を一切参照しない．
//! - **env 注入**: シムの `base_url` を `OPENAI_BASE_URL` に，ダミー鍵を `OPENAI_API_KEY` に注入し，
//!   Python の OpenAI 互換クライアントを pil-llm 単一経路へ routing する（[`SidecarConfig::injected_env`]）．

mod config;

#[cfg(feature = "sidecar")]
mod launcher;

pub use config::{
    SidecarConfig, SidecarRun, DUMMY_API_KEY, ENV_OPENAI_API_KEY, ENV_OPENAI_BASE_URL,
};

#[cfg(feature = "sidecar")]
pub use launcher::{run_sidecar, SidecarError};
