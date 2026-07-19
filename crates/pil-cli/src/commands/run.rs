//! `pil run` — 生成 + 測定（DESIGN §5.5 / §6.2 / §11.3 / IMPLEMENTATION_PLAN M7・M10）．
//!
//! suite TOML を読み，指定ベンチを native 直読し，攻撃バリアント集合を当て，Case × Attack × attempt
//! を `pil-runner` で回す．既定プロバイダは `mock`（network-free）．`ollama` は `--features ollama`
//! でのみ選べる．結果の `Trial` を JSONL で `results/run_<TS>/` に落とし，provenance を同梱する．

use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde_json::json;

use pil_core::Case;
use pil_llm::{LlmConfig, LlmProvider, MockProvider};
use pil_runner::{BoxedInstrument, RunConfig, RunOutcome, Runner};

use crate::commands::{make_results_dir, write_text, Provenance};
use crate::suite::Suite;

/// `pil run` の本体（非同期）．
pub async fn run(repo_root: &Path, suite: &Suite) -> Result<()> {
    // 1. Case を native 直読（§7.3）．ベンチ別に保持し，フラット化して runner へ渡す．
    let case_sets = suite.load_cases(repo_root)?;
    let cases: Vec<Case> = case_sets
        .iter()
        .flat_map(|(_, cs)| cs.iter().cloned())
        .collect();
    if cases.is_empty() {
        bail!("suite `{}` が 1 件も Case を生みませんでした", suite.name);
    }

    // 2. 攻撃バリアントと測定器を解決（§5.6 / §8.2）．
    let attacks = suite.resolve_attacks()?;
    let instruments = suite.resolve_instruments()?;
    let instrument_names: Vec<String> = instruments.iter().map(|i| i.reference().name).collect();

    // 3. 成果物ディレクトリ + チェックポイント（§11.3）．
    let dir = make_results_dir(repo_root, "run")?;
    let checkpoint_path = dir.join("checkpoint.jsonl");

    // 4. 実行設定（多試行 seed 規約は runner 内で `seed_for_attempt`，§11.4）．
    let llm_config = LlmConfig {
        temperature: suite.temperature,
        seed: suite.seed,
        max_tokens: suite.max_tokens,
        system: None,
    };
    let mut config = RunConfig::new(suite.model_ref(), checkpoint_path.clone())
        .attempts(suite.attempts)
        .concurrency(suite.concurrency);
    config.llm_config = llm_config;

    // 5. プロバイダ依存注入 → 実行（§4.1）．
    let outcome = match suite.provider.as_str() {
        "mock" => {
            let provider = Arc::new(MockProvider::new());
            execute(provider, instruments, &cases, &attacks, config).await?
        }
        "ollama" => run_ollama(suite, instruments, &cases, &attacks, config).await?,
        other => bail!("未知のプロバイダ `{other}`（有効: mock / ollama）"),
    };

    // 6. 成果物の書き出し（trials / cases / meta / provenance）．
    write_jsonl(&dir, "trials.jsonl", outcome.trials.iter())?;
    write_jsonl(&dir, "cases.jsonl", cases.iter())?;

    let bench_sizes: Vec<serde_json::Value> = case_sets
        .iter()
        .map(|(name, cs)| json!({ "bench": name, "cases": cs.len() }))
        .collect();
    let meta = json!({
        "suite": suite,
        "provider": suite.provider,
        "model": suite.model,
        "seed": suite.seed,
        "attempts": suite.attempts,
        "n_cases": cases.len(),
        "n_attacks": attacks.len(),
        "instruments": instrument_names,
        "benches": bench_sizes,
        "generated": outcome.generated,
        "skipped": outcome.skipped,
        "errors": outcome.errors.len(),
    });
    write_text(&dir, "run_meta.json", &serde_json::to_string_pretty(&meta)?)?;

    let prov = Provenance::new("run", Some(suite.name.clone()), meta);
    prov.write(&dir)?;

    println!(
        "run 完了: cases={} attacks={} attempts={} → trials={} (生成 {}, スキップ {}, エラー {})",
        cases.len(),
        attacks.len(),
        suite.attempts,
        outcome.trials.len(),
        outcome.generated,
        outcome.skipped,
        outcome.errors.len(),
    );
    println!("成果物: {}", dir.display());
    Ok(())
}

/// 注入済みプロバイダで Runner を回す（mock / ollama 共通経路）．
async fn execute<P>(
    provider: Arc<P>,
    instruments: Vec<BoxedInstrument>,
    cases: &[Case],
    attacks: &[pil_core::AttackRef],
    config: RunConfig,
) -> Result<RunOutcome>
where
    P: LlmProvider + Send + Sync + 'static,
{
    let runner = Runner::new(provider, instruments, config);
    let outcome = runner
        .run(cases, attacks)
        .await
        .context("runner 実行に失敗")?;
    Ok(outcome)
}

/// ollama プロバイダ経路（`--features ollama` 時のみ）．
#[cfg(feature = "ollama")]
async fn run_ollama(
    suite: &Suite,
    instruments: Vec<BoxedInstrument>,
    cases: &[Case],
    attacks: &[pil_core::AttackRef],
    config: RunConfig,
) -> Result<RunOutcome> {
    use pil_llm::backends::ollama::OllamaProvider;
    let endpoint = suite
        .endpoint
        .clone()
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let provider = OllamaProvider::connect(endpoint)
        .await
        .context("ollama への接続に失敗（>= 0.12.11 が必要，DESIGN §6.3）")?;
    execute(Arc::new(provider), instruments, cases, attacks, config).await
}

/// ollama feature 無効時のスタブ（実行時に明示失敗）．
#[cfg(not(feature = "ollama"))]
async fn run_ollama(
    _suite: &Suite,
    _instruments: Vec<BoxedInstrument>,
    _cases: &[Case],
    _attacks: &[pil_core::AttackRef],
    _config: RunConfig,
) -> Result<RunOutcome> {
    bail!("provider=ollama は `--features ollama` でビルドした場合のみ使えます（既定は network-free の mock）")
}

/// serde Serialize な要素群を 1 行 1 JSON（JSONL）で書く．
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
