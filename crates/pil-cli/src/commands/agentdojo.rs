//! `pil agentdojo` — AgentDojo をシム経由でライブ実行する（DESIGN §4.1 / P2-M4b）．
//!
//! **`agentdojo-live` feature でのみコンパイルされる**．既定ビルド（network-free）はこの経路も
//! reqwest/axum/tokio::process も一切引き込まない（§6.1）．
//!
//! 2 モードを持つ:
//!   - **single**（`--limit` 省略）: 1 ケースを実行し `result.json` を残す（従来動作，変更なし）．
//!   - **batch**（`--limit N` 指定）: security ケースを列挙し先頭 N 件を実行し，`EnvKind::Emulated` の
//!     run dir（`cases.jsonl` + `trials.jsonl` + `run_meta.json` + `provenance.json`）を残す．これは
//!     `pil report --run <dir>` がそのまま読める（run.rs の writer を mirror する）．
//!
//! 共通経路（§4.1 の制御反転）:
//!   1. `OpenAiProvider`（ローカル Ollama の OpenAI 互換面へ向ける）を単一プロバイダにする．
//!   2. localhost のエフェメラルポートで pil-shim（OpenAI 互換シム）を **1 回だけ**立てる．
//!   3. シムの base_url を `OPENAI_BASE_URL` に注入して Python 薄殻を sidecar 起動する．
//!      Python の openai SDK は env をそのまま読み，モデル呼び出しはシム＝pil-llm 単一経路へ funnel される．
//!   4. driver の `{utility, security, error}` を注入次元の [`Verdict`](pil_bench_agentdojo::Verdict) に正規化する．

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::json;

use pil_bench_agentdojo::{
    parse_enumeration, parse_result, AgentDojoCase, AgentDojoResult, BENCHMARK_VERSION, COMMIT,
    UPSTREAM,
};
use pil_core::{Case, Trial};
use pil_llm::backends::openai::OpenAiProvider;
use pil_llm::LlmProvider;
use pil_shim::server::{serve, ShimState};
use pil_sidecar::{run_sidecar, SidecarConfig};

use crate::commands::{make_results_dir, write_text, Provenance};

/// `pil agentdojo` の引数（DESIGN §10 / P2-M4b）．
#[derive(Debug, clap::Args)]
pub struct AgentdojoArgs {
    /// AgentDojo suite 名（`banking` / `slack` / `travel` / `workspace`）．
    ///
    /// single モードでは対象 suite として **必須**．batch モードでは filter として働き，省略時は
    /// 列挙された全 suite を対象にする．
    #[arg(long)]
    pub suite: Option<String>,
    /// user task の ID（例 `user_task_0`）．single モードで必須，batch では無視する（列挙で決まる）．
    #[arg(long)]
    pub user_task: Option<String>,
    /// injection task の ID（例 `injection_task_0`）．single モードで必須，batch では無視する．
    #[arg(long)]
    pub injection_task: Option<String>,
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
    /// sidecar 1 件あたりのタイムアウト秒数．
    #[arg(long, default_value_t = 600)]
    pub timeout_secs: u64,
    /// batch モード: 列挙した security ケースの先頭 N 件を回す（未指定は従来の単一ケース実行）．
    #[arg(long)]
    pub limit: Option<usize>,
}

/// `python/run_case.py` の絶対パス（repo_root 相対で確定する）．
fn run_case_script(repo_root: &Path) -> PathBuf {
    repo_root.join("crates/pil-bench-agentdojo/python/run_case.py")
}

/// `python/enumerate_cases.py` の絶対パス（repo_root 相対で確定する）．
fn enumerate_script(repo_root: &Path) -> PathBuf {
    repo_root.join("crates/pil-bench-agentdojo/python/enumerate_cases.py")
}

/// `pil agentdojo` の本体（非同期）．シムを 1 回立て，モードで分岐する．
pub async fn run(repo_root: &Path, args: &AgentdojoArgs) -> Result<()> {
    // 1. 単一プロバイダ（ローカル Ollama の OpenAI 互換面）を Arc<dyn LlmProvider> にする．
    let provider: Arc<dyn LlmProvider> = Arc::new(
        OpenAiProvider::new(args.ollama_base.clone(), args.api_key.clone())
            .with_model_override(args.model.clone()),
    );

    // 2. localhost のエフェメラルポートでシムを **1 回だけ**立てる（§4.1）．single/batch で共有する．
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
    let base_url = format!("http://{addr}/v1");

    // 3. モード分岐．
    match args.limit {
        Some(limit) => run_batch(repo_root, args, &base_url, limit).await,
        None => run_single(repo_root, args, &base_url).await,
    }
}

/// single モード: 1 ケースを実行し `result.json` ＋ provenance を残す（従来動作，変更なし）．
async fn run_single(repo_root: &Path, args: &AgentdojoArgs, base_url: &str) -> Result<()> {
    let suite = args
        .suite
        .as_deref()
        .context("single モードでは --suite が必須です")?;
    let user_task = args
        .user_task
        .as_deref()
        .context("single モードでは --user-task が必須です")?;
    let injection_task = args
        .injection_task
        .as_deref()
        .context("single モードでは --injection-task が必須です")?;

    // driver の {utility, security, error} を注入次元の Verdict に正規化する（§5.3 / §10）．
    let result = run_one_case(repo_root, args, base_url, suite, user_task, injection_task).await?;
    let verdict = result.verdict();

    println!("suite         = {suite} ({user_task} x {injection_task})");
    println!("model         = {} / attack = {}", args.model, args.attack);
    println!("utility       = {}", result.utility);
    println!("security      = {}", result.security);
    println!("verdict       = {verdict:?}");

    // 成果物（result.json ＋ provenance.json）を残す（既存 run.rs の results/ + provenance を踏襲）．
    let dir = make_results_dir(repo_root, "agentdojo")?;
    let verdict_value =
        serde_json::to_value(&verdict).context("Verdict の JSON 化に失敗しました")?;
    let result_json = json!({
        "suite": suite,
        "user_task": user_task,
        "injection_task": injection_task,
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
        "suite": suite,
        "user_task": user_task,
        "injection_task": injection_task,
        "attack": args.attack,
        "model": args.model,
        "adk_model": args.adk_model,
        "ollama_base": args.ollama_base,
        "agentdojo_upstream": UPSTREAM,
        "agentdojo_commit": COMMIT,
        "benchmark_version": BENCHMARK_VERSION,
    });
    let prov = Provenance::new("agentdojo", Some(suite.to_string()), params);
    prov.write(&dir)?;

    println!("成果物: {}", dir.display());
    Ok(())
}

/// batch モード: security ケースを列挙し先頭 N 件を実行し，`EnvKind::Emulated` の run dir を残す．
///
/// 1 件が失敗しても batch は止めず，その件を `Undecidable`（error）として集計へ含める（§3.6）．
async fn run_batch(
    repo_root: &Path,
    args: &AgentdojoArgs,
    base_url: &str,
    limit: usize,
) -> Result<()> {
    // 1. ケース列挙（network-free；agentdojo を import するだけ，モデルは呼ばない）．
    let all = enumerate_all_cases(repo_root, args, base_url).await?;
    let filtered: Vec<AgentDojoCase> = match &args.suite {
        Some(s) => all.into_iter().filter(|c| &c.suite == s).collect(),
        None => all,
    };
    if filtered.is_empty() {
        bail!(
            "列挙結果が空です（suite filter = {:?}）．suite 名・agentdojo インストールを確認してください",
            args.suite
        );
    }
    // 決定論的順序で先頭 N 件を選ぶ（列挙順は Python 側で suite→user→injection の巡回で安定）．
    let selected: Vec<AgentDojoCase> = filtered.into_iter().take(limit).collect();
    let n = selected.len();
    eprintln!(
        "agentdojo batch: 列挙から {n} 件を実行します（limit={limit}, suite={:?}, attack={}, model={}）",
        args.suite, args.attack, args.model
    );

    // 2. 各ケースを実行し Case / Trial を積む（per-case エラーは潰さず継続）．
    let mut cases: Vec<Case> = Vec::with_capacity(n);
    let mut trials: Vec<Trial> = Vec::with_capacity(n);
    let mut n_security = 0usize;
    let mut n_errored = 0usize;

    for (i, adc) in selected.iter().enumerate() {
        let case = adc.to_case();
        let result = match run_one_case(
            repo_root,
            args,
            base_url,
            &adc.suite,
            &adc.user_task_id,
            &adc.injection_task_id,
        )
        .await
        {
            Ok(r) => r,
            // sidecar 失敗 / parse 失敗は Undecidable として記録し batch は継続する（§3.6）．
            Err(e) => AgentDojoResult::errored(format!("{e:#}")),
        };

        let is_error = result
            .error
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if is_error {
            n_errored += 1;
        } else if result.security {
            n_security += 1;
        }
        eprintln!(
            "[{}/{n}] {}/{}/{} → security={} utility={} error={:?}",
            i + 1,
            adc.suite,
            adc.user_task_id,
            adc.injection_task_id,
            result.security,
            result.utility,
            result.error,
        );

        trials.push(result.to_trial(&case));
        cases.push(case);
    }

    // 3. Emulated run dir を書く（run.rs の writer を mirror し，report がそのまま読める）．
    let dir = make_results_dir(repo_root, "agentdojo_batch")?;
    write_jsonl(&dir, "trials.jsonl", trials.iter())?;
    write_jsonl(&dir, "cases.jsonl", cases.iter())?;

    let meta = json!({
        "mode": "batch",
        "env_kind": "Emulated",
        "suite_filter": args.suite,
        "limit": limit,
        "attack": args.attack,
        "model": args.model,
        "adk_model": args.adk_model,
        "ollama_base": args.ollama_base,
        "n_cases": n,
        "n_security_success": n_security,
        "n_errored": n_errored,
        "agentdojo_upstream": UPSTREAM,
        "agentdojo_commit": COMMIT,
        "benchmark_version": BENCHMARK_VERSION,
    });
    write_text(&dir, "run_meta.json", &serde_json::to_string_pretty(&meta)?)?;

    let prov = Provenance::new("agentdojo_batch", args.suite.clone(), meta);
    prov.write(&dir)?;

    println!(
        "agentdojo batch 完了: cases={n} security-success={n_security}（Emulated ASR 分子）errored={n_errored}"
    );
    println!("成果物: {}", dir.display());
    Ok(())
}

/// `enumerate_cases.py` を sidecar 起動し，`Vec<AgentDojoCase>` を得る（network-free）．
async fn enumerate_all_cases(
    repo_root: &Path,
    args: &AgentdojoArgs,
    base_url: &str,
) -> Result<Vec<AgentDojoCase>> {
    let script = enumerate_script(repo_root);
    if !script.is_file() {
        bail!("enumerate_cases.py が見つかりません: {}", script.display());
    }
    let config = SidecarConfig::new(&args.python, script, base_url.to_string())
        .with_args(["--benchmark-version", BENCHMARK_VERSION]);
    let sidecar = run_sidecar(&config, Duration::from_secs(args.timeout_secs))
        .await
        .with_context(|| {
            format!(
                "enumerate_cases.py の起動に失敗しました（python `{}` や agentdojo venv を確認してください）",
                args.python
            )
        })?;
    if !sidecar.success || sidecar.stdout.trim().is_empty() {
        bail!(
            "enumerate_cases.py が異常終了しました（exit={:?}）: {}",
            sidecar.exit_code,
            sidecar.stderr
        );
    }
    parse_enumeration(&sidecar.stdout).with_context(|| {
        format!(
            "enumerate_cases.py の出力 JSON を解釈できません: {}",
            sidecar.stdout
        )
    })
}

/// `run_case.py` を sidecar 起動し，`AgentDojoResult` を得る（single/batch 共通）．
async fn run_one_case(
    repo_root: &Path,
    args: &AgentdojoArgs,
    base_url: &str,
    suite: &str,
    user_task: &str,
    injection_task: &str,
) -> Result<AgentDojoResult> {
    let script = run_case_script(repo_root);
    if !script.is_file() {
        bail!("run_case.py が見つかりません: {}", script.display());
    }
    let config = SidecarConfig::new(&args.python, script, base_url.to_string()).with_args([
        "--suite",
        suite,
        "--user-task",
        user_task,
        "--injection-task",
        injection_task,
        "--attack",
        args.attack.as_str(),
        "--model",
        args.adk_model.as_str(),
    ]);

    let sidecar = run_sidecar(&config, Duration::from_secs(args.timeout_secs))
        .await
        .with_context(|| {
            format!(
                "sidecar の起動に失敗しました（python `{}` や agentdojo venv を確認してください）",
                args.python
            )
        })?;
    if !sidecar.success || sidecar.stdout.trim().is_empty() {
        bail!(
            "run_case.py が異常終了しました（exit={:?}）．Ollama 到達性（{}）・agentdojo venv・モデル `{}` の tool-calling 対応を確認してください．stderr: {}",
            sidecar.exit_code,
            args.ollama_base,
            args.model,
            sidecar.stderr,
        );
    }

    parse_result(&sidecar.stdout).with_context(|| {
        format!(
            "run_case.py の出力 JSON を解釈できません: {}",
            sidecar.stdout
        )
    })
}

/// serde Serialize な要素群を 1 行 1 JSON（JSONL）で書く（run.rs の writer を mirror する）．
fn write_jsonl<'a, T, I>(dir: &Path, name: &str, items: I) -> Result<()>
where
    T: serde::Serialize + 'a,
    I: Iterator<Item = &'a T>,
{
    use std::fmt::Write as _;
    let mut buf = String::new();
    for item in items {
        let line = serde_json::to_string(item)?;
        writeln!(buf, "{line}").expect("write to String");
    }
    write_text(dir, name, &buf)?;
    Ok(())
}
