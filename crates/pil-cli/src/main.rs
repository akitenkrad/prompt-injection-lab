//! `pil` — prompt-injection-lab の CLI（DESIGN §10 / IMPLEMENTATION_PLAN M10）．
//!
//! サブコマンド:
//! - `reliability`: judge 信頼性の開示（§3.1 の再現，network-free）．
//! - `run`: suite に従い Case × Attack × attempt を生成 + 測定（既定 `--provider mock`）．
//! - `report`: run の成果物から単発 ASR / union / 多試行 / 過剰拒否 / 非独立性を提示（§8.1）．
//!
//! 既定ビルドはネットワーク非依存（§6.1）．`ollama` プロバイダは `--features ollama` でのみ有効．

mod commands;
mod judges;
mod repo;
mod suite;
mod timestamp;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::repo::resolve_repo_root;
use crate::suite::Suite;

/// prompt-injection-lab CLI．
#[derive(Debug, Parser)]
#[command(
    name = "pil",
    version,
    about = "prompt-injection-lab: 横断比較の測定基盤"
)]
struct Cli {
    /// `third_party/` を含むリポジトリルート（省略時は CWD から上方探索）．
    #[arg(long, global = true)]
    repo_root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// judge 信頼性を開示する（§3.1 の再現，LLM/ネットワーク不使用）．
    Reliability,
    /// suite を実行して Trial を生成する（生成 + 測定）．
    Run {
        /// suite TOML のパス（例: `suites/phase1-smoke.toml`）．
        #[arg(long)]
        suite: PathBuf,
        /// プロバイダを上書きする（`mock` / `ollama`）．省略時は suite の値．
        #[arg(long)]
        provider: Option<String>,
    },
    /// run の成果物から集計レポートを出す（§8.1）．
    Report {
        /// 対象の run ディレクトリ（`results/run_*`）．
        #[arg(long)]
        run: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let repo_root = resolve_repo_root(cli.repo_root.clone());

    match cli.command {
        Command::Reliability => commands::reliability::run(&repo_root),
        Command::Run { suite, provider } => {
            let mut parsed = Suite::load(&suite)?;
            if let Some(p) = provider {
                parsed.provider = p;
            }
            // 生成は非同期．現在スレッドのランタイムで回す（既定 mock は I/O を持たない）．
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(commands::run::run(&repo_root, &parsed))
        }
        Command::Report { run } => commands::report::run(&repo_root, &run),
    }
}
