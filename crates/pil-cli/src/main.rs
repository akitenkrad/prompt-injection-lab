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
        /// 対象の run ディレクトリ（`results/run_*` / `results/agentdojo_batch_*`）．
        /// 繰り返し指定でき（`--run A --run B`），複数指定した run は union（連結）して集計する．
        #[arg(long, required = true)]
        run: Vec<PathBuf>,
        /// EnvKind 跨ぎの明示開示（プール ASR + 警告 + Kendall W）を出す（§8.1 / §3.7）．
        /// 既定（未指定）では横断スカラを一切出さない．
        #[arg(long, default_value_t = false)]
        cross_env: bool,
    },
    /// AgentDojo 1 ケースをシム経由でライブ実行する（feature `agentdojo-live`）．
    #[cfg(feature = "agentdojo-live")]
    Agentdojo(commands::agentdojo::AgentdojoArgs),
    /// StrongREJECT fine-tuned judge を python sidecar 経由で回す（feature `strongreject-judge`）．
    #[cfg(feature = "strongreject-judge")]
    StrongrejectJudge(commands::strongreject_judge::StrongrejectJudgeArgs),
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
        Command::Report { run, cross_env } => commands::report::run(&repo_root, &run, cross_env),
        #[cfg(feature = "agentdojo-live")]
        Command::Agentdojo(args) => {
            // ライブ実行は非同期（シム起動 + sidecar 起動）．現在スレッドの外に multi-thread ランタイムを立てる．
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(commands::agentdojo::run(&repo_root, &args))
        }
        // sidecar は std::process で同期起動するため tokio ランタイムは不要．
        #[cfg(feature = "strongreject-judge")]
        Command::StrongrejectJudge(args) => commands::strongreject_judge::run(&repo_root, &args),
    }
}
