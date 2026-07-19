//! `pil report` — 集計 + 提示（DESIGN §3.4 / §5.3 / §8.1 / §10 / IMPLEMENTATION_PLAN M8・M9・M10）．
//!
//! `results/run_*/` の `trials.jsonl` / `cases.jsonl` を読み，次を **常に信頼区間 + undecidable 件数
//! つき**で出す（§8.1）:
//!
//! - 単発 ASR（EnvKind ごと・測定器ごと，Wilson CI）
//! - union coverage（単一最良 vs union，§2.2）
//! - 多試行 ASR 曲線（1/10/100 as available，§11.3）
//! - 過剰拒否（JBB benign，§10）
//! - ベンチ横断の ContentKey 重複 = 非独立性（§3.4，上流全件で算出）
//!
//! さらに **Phase 1 は全て `StaticPrompt` のため EnvKind 跨ぎ比較は行わない**ことを明示する（§10）．
//! 成果物は `results/report_<TS>/` に human-readable + machine-readable JSON で保存する．

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::json;

use pil_core::{Case, EnvKind, Trial, Verdict};
use pil_metrics::aggregate::{
    asr_curve_by_env, single_shot_asr_by_env, union_coverage_by_env, wilson_interval, Z_95,
};
use pil_report::{detect_duplicates, AsrPresentation, DUPLICATE_METHOD};

use crate::commands::{make_results_dir, write_text, Provenance};
use crate::suite::load_bench;

/// bootstrap の反復回数（決定論・seeded，§11.3）．
const N_BOOT: usize = 1000;
/// bootstrap の有意水準（95% CI）．
const ALPHA: f64 = 0.05;
/// bootstrap の seed（決定論）．
const BOOT_SEED: u64 = 0xB007;
/// RefusalMatch 測定器名（過剰拒否の判定に使う）．
const REFUSAL_MATCH: &str = "advbench-refusal-match";

/// `pil report` の本体．
pub fn run(repo_root: &Path, run_dir: &Path) -> Result<()> {
    let trials = read_trials(run_dir)?;
    let cases = read_cases(run_dir)?;
    if trials.is_empty() {
        bail!(
            "run ディレクトリに trials が 1 件もありません: {}",
            run_dir.display()
        );
    }

    // CaseId(short) -> Case（env / benign を引く）．
    let case_index: BTreeMap<String, Case> = cases
        .iter()
        .map(|c| (c.id.short().to_string(), c.clone()))
        .collect();

    // EnvKind ごとに Trial を分ける（§8.1 の型強制入力）．
    let mut by_env: BTreeMap<EnvKind, Vec<Trial>> = BTreeMap::new();
    for t in &trials {
        let env = case_index
            .get(t.case.short())
            .map(|c| c.env_kind)
            .unwrap_or(EnvKind::StaticPrompt);
        by_env.entry(env).or_default().push(t.clone());
    }

    let max_attempt = trials.iter().map(|t| t.attempt).max().unwrap_or(1);
    let ks = multi_trial_ks(max_attempt);

    let mut out = String::new();
    writeln!(out, "# pil report（{}）", run_dir.display())?;
    writeln!(out, "trials={} cases={}", trials.len(), cases.len())?;

    // --- Phase 1 の割り切り（§10）を明示する ---
    let envs: Vec<EnvKind> = by_env.keys().copied().collect();
    writeln!(out, "\n## 環境種別（EnvKind）")?;
    writeln!(out, "観測された EnvKind: {envs:?}")?;
    if envs.len() <= 1 {
        writeln!(
            out,
            "→ Phase 1 の全ベンチは StaticPrompt．環境種別が 1 種のため，\
             EnvKind 跨ぎの比較は行わない（§10 / §8.1）．Kendall W=0.10 の比較不能性の核心は Phase 2．"
        )?;
    } else {
        writeln!(
            out,
            "→ 複数 EnvKind が混在．集計は EnvKind ごとに分離し，跨いだ単一スコアは出さない（§8.1）．"
        )?;
    }

    // --- 単発 ASR（EnvKind ごと・測定器ごと，Wilson CI，§8.1） ---
    writeln!(
        out,
        "\n## 単発 ASR（測定器ごと・Wilson 95% CI・undecidable 併記）"
    )?;
    let single = single_shot_asr_by_env(&by_env, Z_95);
    let mut single_json = Vec::new();
    for (env, env_asr) in &single {
        writeln!(out, "### EnvKind = {env:?}")?;
        for (inst, res) in &env_asr.per_instrument {
            let pres = AsrPresentation::from_asr_result(res, 95.0);
            writeln!(out, "  [{} {}] {}", inst.name, inst.version, pres.render())?;
            single_json.push(json!({
                "env": format!("{env:?}"),
                "instrument": inst.name,
                "version": inst.version,
                "presentation": pres,
            }));
        }
    }

    // --- union coverage（単一最良 vs union，§2.2） ---
    writeln!(
        out,
        "\n## union coverage（単一最良 vs 攻撃バリアント union，§2.2）"
    )?;
    let union = union_coverage_by_env(&by_env, N_BOOT, ALPHA, BOOT_SEED);
    let mut union_json = Vec::new();
    for (env, ucs) in &union {
        writeln!(out, "### EnvKind = {env:?}")?;
        for uc in ucs {
            let best = fmt_pct(uc.single_best_asr);
            let uni = fmt_pct(uc.union_asr);
            writeln!(
                out,
                "  [{} {}] 単一最良={best} / union={uni} \
                 [{:.1}%, {:.1}%] (n_cases={}, undecidable-only={})",
                uc.instrument.name,
                uc.instrument.version,
                uc.union_ci.lower * 100.0,
                uc.union_ci.upper * 100.0,
                uc.n_cases,
                uc.undecidable_only_cases,
            )?;
            union_json.push(json!({
                "env": format!("{env:?}"),
                "instrument": uc.instrument.name,
                "version": uc.instrument.version,
                "single_best_asr": uc.single_best_asr,
                "union_asr": uc.union_asr,
                "union_ci_lower": uc.union_ci.lower,
                "union_ci_upper": uc.union_ci.upper,
                "n_cases": uc.n_cases,
                "undecidable_only_cases": uc.undecidable_only_cases,
            }));
        }
    }

    // --- 多試行 ASR 曲線（1/10/100 as available，§11.3） ---
    writeln!(
        out,
        "\n## 多試行 ASR 曲線（asr@k，k={ks:?}，Case 単位 bootstrap CI）"
    )?;
    let curves = asr_curve_by_env(&by_env, &ks, N_BOOT, ALPHA, BOOT_SEED);
    let mut curve_json = Vec::new();
    for (env, cs) in &curves {
        writeln!(out, "### EnvKind = {env:?}")?;
        for curve in cs {
            let pts: Vec<String> = curve
                .points
                .iter()
                .map(|p| format!("@{}={}", p.k, fmt_pct(p.asr)))
                .collect();
            writeln!(
                out,
                "  [{} {}] {} (n_cases={}, undecidable-only={})",
                curve.instrument.name,
                curve.instrument.version,
                pts.join(" "),
                curve.n_cases,
                curve.undecidable_only_cases,
            )?;
            curve_json.push(json!({
                "env": format!("{env:?}"),
                "instrument": curve.instrument.name,
                "version": curve.instrument.version,
                "points": curve.points,
                "n_cases": curve.n_cases,
                "undecidable_only_cases": curve.undecidable_only_cases,
            }));
        }
    }

    // --- 過剰拒否（JBB benign，§10 / 設計指針4） ---
    writeln!(out, "\n## 過剰拒否（JBB benign 上での拒否率，§10）")?;
    let over = over_refusal(&trials, &case_index);
    let over_json = match &over {
        Some((pres, n_benign)) => {
            writeln!(
                out,
                "  benign 判定対象={n_benign}: 過剰拒否率 {}（RefusalMatch が拒否と判定した割合）",
                pres.render()
            )?;
            json!({ "n_benign_decided": n_benign, "presentation": pres })
        }
        None => {
            writeln!(
                out,
                "  benign な Case × RefusalMatch の測定が無いため過剰拒否は算出できない\
                 （suite に jbb_benign と refusal_match を含めること）．"
            )?;
            json!(null)
        }
    };

    // --- ベンチ横断の非独立性（§3.4，上流全件で算出） ---
    writeln!(out, "\n## ベンチ間の非独立性（ContentKey 重複，§3.4）")?;
    writeln!(
        out,
        "（run のサンプルとは独立に，上流全件 AdvBench/HarmBench/JBB-harmful で算出，方式: {DUPLICATE_METHOD}）"
    )?;
    let dup_json = match duplicates_full(repo_root) {
        Ok((rendered, value)) => {
            out.push_str(&indent(&rendered, "  "));
            value
        }
        Err(e) => {
            writeln!(out, "  重複検出をスキップ（上流ロード失敗）: {e:#}")?;
            json!(null)
        }
    };

    // 標準出力にも要約を出す．
    println!("{out}");

    // 成果物保存（human-readable + machine-readable）．
    let dir = make_results_dir(repo_root, "report")?;
    write_text(&dir, "report.txt", &out)?;
    let machine = json!({
        "run_dir": run_dir.display().to_string(),
        "n_trials": trials.len(),
        "n_cases": cases.len(),
        "env_kinds": envs.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>(),
        "phase1_cross_env_comparison": false,
        "ks": ks,
        "single_shot_asr": single_json,
        "union_coverage": union_json,
        "asr_curve": curve_json,
        "over_refusal": over_json,
        "duplicates": dup_json,
    });
    write_text(
        &dir,
        "report.json",
        &serde_json::to_string_pretty(&machine)?,
    )?;

    let prov = Provenance::new(
        "report",
        None,
        json!({
            "run_dir": run_dir.display().to_string(),
            "n_trials": trials.len(),
            "ks": ks,
        }),
    );
    prov.write(&dir)?;

    println!("成果物: {}", dir.display());
    Ok(())
}

/// 多試行 ASR 曲線の k 集合（{1,10,100} ∩ [1, max_attempt] に max_attempt を加える）．
fn multi_trial_ks(max_attempt: u32) -> Vec<u32> {
    let mut ks: Vec<u32> = [1u32, 10, 100]
        .into_iter()
        .filter(|&k| k <= max_attempt)
        .collect();
    if !ks.contains(&max_attempt) {
        ks.push(max_attempt);
    }
    ks.sort_unstable();
    ks.dedup();
    ks
}

/// 過剰拒否（benign Case × RefusalMatch）を算出する（§10）．
///
/// `RefusalMatch::Failure`（= 拒否接頭辞が応答に現れた）を「拒否」と数え，`Success`（拒否なし）を
/// 「応答した」と数える．`Undecidable` は分母から除外する（§5.3）．戻り値は提示と判定対象件数．
fn over_refusal(
    trials: &[Trial],
    case_index: &BTreeMap<String, Case>,
) -> Option<(AsrPresentation, usize)> {
    let (mut refused, mut complied) = (0usize, 0usize);
    for t in trials {
        let is_benign = case_index
            .get(t.case.short())
            .map(|c| c.benign)
            .unwrap_or(false);
        if !is_benign {
            continue;
        }
        for m in &t.measurements {
            if m.instrument.name != REFUSAL_MATCH {
                continue;
            }
            match m.verdict {
                Verdict::Failure => refused += 1, // 拒否接頭辞あり = 過剰拒否
                Verdict::Success => complied += 1,
                Verdict::Undecidable { .. } => {}
            }
        }
    }
    let decided = refused + complied;
    if decided == 0 {
        return None;
    }
    let ci = wilson_interval(refused, decided, Z_95);
    let pres = AsrPresentation {
        asr: refused as f64 / decided as f64,
        ci_lower: ci.lower,
        ci_upper: ci.upper,
        confidence_pct: 95.0,
        successes: refused,
        decided,
        undecidable: 0,
    };
    Some((pres, decided))
}

/// 上流全件（AdvBench / HarmBench text / JBB-harmful）で §3.4 の重複を算出する．
fn duplicates_full(repo_root: &Path) -> Result<(String, serde_json::Value)> {
    let advbench = load_bench("advbench", repo_root)?;
    let harmbench = load_bench("harmbench_text", repo_root)?;
    let jbb = load_bench("jbb_harmful", repo_root)?;
    let sets: Vec<(&str, &[Case])> = vec![
        ("JBB", jbb.as_slice()),
        ("AdvBench", advbench.as_slice()),
        ("HarmBench", harmbench.as_slice()),
    ];
    let report = detect_duplicates(&sets);
    let value = serde_json::to_value(&report)?;
    Ok((report.render(), value))
}

/// `trials.jsonl` を読む．
fn read_trials(run_dir: &Path) -> Result<Vec<Trial>> {
    read_jsonl(&run_dir.join("trials.jsonl"))
}

/// `cases.jsonl` を読む．
fn read_cases(run_dir: &Path) -> Result<Vec<Case>> {
    read_jsonl(&run_dir.join("cases.jsonl"))
}

/// JSONL を型付きで読む（空行は無視）．
fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("JSONL を読めません: {}", path.display()))?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let item: T = serde_json::from_str(line)
            .with_context(|| format!("{} 行 {} のパースに失敗", path.display(), i + 1))?;
        out.push(item);
    }
    Ok(out)
}

/// 割合を `NaN` 安全に `%` 文字列へ．
fn fmt_pct(v: f64) -> String {
    if v.is_nan() {
        "N/A".to_string()
    } else {
        format!("{:.1}%", v * 100.0)
    }
}

/// 複数行文字列に接頭辞を付ける（重複レポートの字下げ）．
fn indent(text: &str, prefix: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(prefix);
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ks_includes_one_and_max() {
        assert_eq!(multi_trial_ks(1), vec![1]);
        assert_eq!(multi_trial_ks(3), vec![1, 3]);
        assert_eq!(multi_trial_ks(10), vec![1, 10]);
        assert_eq!(multi_trial_ks(100), vec![1, 10, 100]);
        assert_eq!(multi_trial_ks(50), vec![1, 10, 50]);
    }
}
