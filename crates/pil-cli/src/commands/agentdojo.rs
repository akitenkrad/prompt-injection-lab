//! `pil agentdojo` — AgentDojo 1 ケースをシム経由でライブ実行する（DESIGN §4.1 / P2-M4b）．
//!
//! **`agentdojo-live` feature でのみコンパイルされる**．既定ビルド（network-free）はこの経路も
//! reqwest/axum/tokio::process も一切引き込まない（§6.1）．
//!
//! 経路（§4.1 の制御反転）:
//!   1. `OpenAiProvider`（ローカル Ollama の OpenAI 互換面へ向ける）を単一プロバイダにする．
//!   2. localhost のエフェメラルポートで pil-shim（OpenAI 互換シム）を立てる．
//!   3. シムの base_url を `OPENAI_BASE_URL` に注入して `python/run_case.py` を sidecar 起動する．
//!      Python の openai SDK は env をそのまま読み，モデル呼び出しはシム＝pil-llm 単一経路へ funnel される．
//!   4. driver の `{utility, security, error}` を注入次元の [`Verdict`](pil_bench_agentdojo::Verdict) に正規化し，
//!      `results/agentdojo_<TS>/` に result.json ＋ provenance.json を残す．

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::json;

use pil_bench_agentdojo::{parse_result, BENCHMARK_VERSION, COMMIT, UPSTREAM};
use pil_llm::backends::openai::OpenAiProvider;
use pil_llm::LlmProvider;
use pil_shim::server::{serve, ShimState};
use pil_sidecar::{run_sidecar, SidecarConfig};

use crate::commands::{make_results_dir, write_text, Provenance};

/// `pil agentdojo` の引数（DESIGN §10 / P2-M4b）．
#[derive(Debug, clap::Args)]
pub struct AgentdojoArgs {
    /// AgentDojo suite 名（`banking` / `slack` / `travel` / `workspace`）．
    #[arg(long)]
    pub suite: String,
    /// user task の ID（例 `user_task_0`）．
    #[arg(long)]
    pub user_task: String,
    /// injection task の ID（例 `injection_task_0`）．
    #[arg(long)]
    pub injection_task: String,
    /// 攻撃名（AgentDojo の attack registry）．
    #[arg(long, default_value = "important_instructions")]
    pub attack: String,
    /// 実際に呼び出すローカル Ollama のモデルタグ（シム境界で送信モデル名を上書きする）．
    #[arg(long, default_value = "gpt-oss:20b")]
    pub model: String,
    /// AgentDojo に渡すモデル名（有効な `ModelsEnum`．provider=openai の native tool-calling を使う）．
    ///
    /// AgentDojo は未知のモデル名を弾くため，enum に存在する OpenAI プロバイダのモデルを指定し，
    /// 実タグ（`--model`）への差し替えはシム境界の `OpenAiProvider` が行う（§4.1）．
    #[arg(long, default_value = "gpt-4o-2024-05-13")]
    pub adk_model: String,
    /// Ollama の OpenAI 互換 base_url（`/v1` を含む）．
    #[arg(long, default_value = "http://localhost:11434/v1")]
    pub ollama_base: String,
    /// プロバイダへ送る API 鍵（Ollama はダミーで可）．
    #[arg(long, default_value = "ollama")]
    pub api_key: String,
    /// agentdojo 用 Python インタプリタ．
    #[arg(long, default_value = ".venv-agentdojo/bin/python")]
    pub python: String,
    /// sidecar のタイムアウト秒数．
    #[arg(long, default_value_t = 600)]
    pub timeout_secs: u64,
}

/// `python/run_case.py` の絶対パス（repo_root 相対で確定する）．
fn run_case_script(repo_root: &Path) -> PathBuf {
    repo_root.join("crates/pil-bench-agentdojo/python/run_case.py")
}

/// `pil agentdojo` の本体（非同期）．
pub async fn run(repo_root: &Path, args: &AgentdojoArgs) -> Result<()> {
    // 1. 単一プロバイダ（ローカル Ollama の OpenAI 互換面）を Arc<dyn LlmProvider> にする．
    let provider: Arc<dyn LlmProvider> = Arc::new(
        OpenAiProvider::new(args.ollama_base.clone(), args.api_key.clone())
            .with_model_override(args.model.clone()),
    );

    // 2. localhost のエフェメラルポートでシムを立てる（§4.1）．
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("シム用エフェメラルポートの bind に失敗しました")?;
    let addr = listener
        .local_addr()
        .context("シムの local_addr 取得に失敗しました")?;
    let state = Arc::new(ShimState::new(provider, vec![args.model.clone()]));
    tokio::spawn(async move {
        let _ = serve(listener, state).await;
    });

    // 3. シムの base_url を注入して run_case.py を sidecar 起動する（モデル呼び出しは env でシムへ）．
    let base_url = format!("http://{addr}/v1");
    let script = run_case_script(repo_root);
    if !script.is_file() {
        bail!("run_case.py が見つかりません: {}", script.display());
    }
    let config = SidecarConfig::new(&args.python, script, base_url.clone()).with_args([
        "--suite",
        args.suite.as_str(),
        "--user-task",
        args.user_task.as_str(),
        "--injection-task",
        args.injection_task.as_str(),
        "--attack",
        args.attack.as_str(),
        "--model",
        args.adk_model.as_str(),
    ]);

    // 4. sidecar 実行（Ollama 到達不能・python/venv 不在はここで明示失敗させる）．
    let sidecar = run_sidecar(&config, Duration::from_secs(args.timeout_secs))
        .await
        .with_context(|| {
            format!(
                "sidecar の起動に失敗しました（python `{}` や agentdojo venv を確認してください）",
                args.python
            )
        })?;
    if !sidecar.success || sidecar.stdout.trim().is_empty() {
        eprintln!("--- run_case.py stderr ---\n{}", sidecar.stderr);
        bail!(
            "run_case.py が異常終了しました（exit={:?}）．Ollama 到達性（{}）・agentdojo venv・モデル `{}` の tool-calling 対応を確認してください",
            sidecar.exit_code,
            args.ollama_base,
            args.model
        );
    }

    // 5. driver の {utility, security, error} を注入次元の Verdict に正規化する（§5.3 / §10）．
    let result = parse_result(&sidecar.stdout).with_context(|| {
        format!(
            "run_case.py の出力 JSON を解釈できません: {}",
            sidecar.stdout
        )
    })?;
    let verdict = result.verdict();

    println!(
        "suite         = {} ({} x {})",
        args.suite, args.user_task, args.injection_task
    );
    println!("model         = {} / attack = {}", args.model, args.attack);
    println!("utility       = {}", result.utility);
    println!("security      = {}", result.security);
    println!("verdict       = {verdict:?}");

    // 6. 成果物（result.json ＋ provenance.json）を残す（既存 run.rs の results/ + provenance を踏襲）．
    let dir = make_results_dir(repo_root, "agentdojo")?;
    let verdict_value =
        serde_json::to_value(&verdict).context("Verdict の JSON 化に失敗しました")?;
    let result_json = json!({
        "suite": args.suite,
        "user_task": args.user_task,
        "injection_task": args.injection_task,
        "attack": args.attack,
        "model": args.model,
        "utility": result.utility,
        "security": result.security,
        "verdict": verdict_value,
    });
    write_text(
        &dir,
        "result.json",
        &serde_json::to_string_pretty(&result_json)?,
    )?;

    let params = json!({
        "suite": args.suite,
        "user_task": args.user_task,
        "injection_task": args.injection_task,
        "attack": args.attack,
        "model": args.model,
        "adk_model": args.adk_model,
        "ollama_base": args.ollama_base,
        "agentdojo_upstream": UPSTREAM,
        "agentdojo_commit": COMMIT,
        "benchmark_version": BENCHMARK_VERSION,
    });
    let prov = Provenance::new("agentdojo", Some(args.suite.clone()), params);
    prov.write(&dir)?;

    println!("成果物: {}", dir.display());
    Ok(())
}
