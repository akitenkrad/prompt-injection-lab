//! suite TOML の定義・パース・解決（DESIGN §10 / IMPLEMENTATION_PLAN M10）．
//!
//! 実験セットを 1 つの TOML で宣言する．ベンチ（件数上限つき）・攻撃バリアント・測定器・
//! プロバイダ・試行回数・seed を持つ．`toml` でパースし，`pil-bench-*` / `pil-attacks` /
//! `pil-metrics::instrument` の実 API へ解決する．

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use pil_attacks::{
    base64, default_variants, identity, leetspeak, refusal_suppression, roleplay, translate,
};
use pil_core::{AttackRef, Case, ModelRef};
use pil_metrics::instrument::{HarmBenchCls, RefusalMatch, Rubric};
use pil_runner::BoxedInstrument;

use crate::judges::{canned_cls_judge, canned_rubric_judge, CannedJudge};

/// 実験セット定義（suite TOML のルート）．
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Suite {
    /// スイート名（成果物 provenance に記録）．
    pub name: String,
    /// 説明（任意）．
    #[serde(default)]
    pub description: String,
    /// 基底 seed（多試行 seed 規約 `seed_for_attempt` の基底，§11.4）．
    #[serde(default)]
    pub seed: u64,
    /// 生成プロバイダ: `mock`（既定・network-free）または `ollama`（`--features ollama`）．
    #[serde(default = "default_provider")]
    pub provider: String,
    /// モデル名（mock では表示のみ，ollama では実モデル ID）．
    #[serde(default = "default_model")]
    pub model: String,
    /// エンドポイント（ollama 用．例 `http://localhost:11434`）．
    #[serde(default)]
    pub endpoint: Option<String>,
    /// 多試行回数（1..=N，Anthropic 式 1/10/100，§6.2）．
    #[serde(default = "default_attempts")]
    pub attempts: u32,
    /// 有界並行の同時実行数（§11.3）．
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// サンプリング温度（多試行は >0 が望ましい，§11.4）．
    #[serde(default)]
    pub temperature: f32,
    /// 生成上限トークン（None ならプロバイダ既定）．
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// 攻撃バリアント（`["default"]` で `pil_attacks::default_variants`，§5.6）．
    #[serde(default = "default_attacks")]
    pub attacks: Vec<String>,
    /// 測定器（`refusal_match` / `harmbench_cls` / `rubric_v1` / `rubric_v2`，§8.2）．
    #[serde(default = "default_instruments")]
    pub instruments: Vec<String>,
    /// 対象ベンチ（件数上限つき）．
    #[serde(default)]
    pub benches: Vec<BenchSpec>,
}

/// 1 ベンチの指定．
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BenchSpec {
    /// ベンチ名（[`BENCH_NAMES`] のいずれか）．
    pub name: String,
    /// 先頭からの件数上限（None なら全件）．
    #[serde(default)]
    pub limit: Option<usize>,
}

fn default_provider() -> String {
    "mock".to_string()
}
fn default_model() -> String {
    "mock-1".to_string()
}
fn default_attempts() -> u32 {
    1
}
fn default_concurrency() -> usize {
    4
}
fn default_attacks() -> Vec<String> {
    vec!["default".to_string()]
}
fn default_instruments() -> Vec<String> {
    vec!["refusal_match".to_string()]
}

/// 使用可能なベンチ名（suite の `benches[].name`）．
pub const BENCH_NAMES: &[&str] = &[
    "advbench",
    "harmbench_text",
    "harmbench_multimodal",
    "strongreject_full",
    "strongreject_small",
    "jbb_harmful",
    "jbb_benign",
];

impl Suite {
    /// TOML 文字列からパースする．
    pub fn from_toml_str(s: &str) -> Result<Self> {
        let suite: Suite = toml::from_str(s).context("suite TOML のパースに失敗")?;
        suite.validate()?;
        Ok(suite)
    }

    /// ファイルから読み込む．
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("suite ファイルを読めません: {}", path.display()))?;
        Self::from_toml_str(&text)
    }

    fn validate(&self) -> Result<()> {
        if self.benches.is_empty() {
            bail!("suite `{}` に benches が 1 つもありません", self.name);
        }
        for b in &self.benches {
            if !BENCH_NAMES.contains(&b.name.as_str()) {
                bail!(
                    "未知のベンチ `{}`（有効: {}）",
                    b.name,
                    BENCH_NAMES.join(", ")
                );
            }
        }
        // 解決可能性を先に検査（実行前に失敗させる）．
        self.resolve_attacks()?;
        self.resolve_instrument_specs()?;
        Ok(())
    }

    /// RunConfig 用の `ModelRef` を組む．
    pub fn model_ref(&self) -> ModelRef {
        ModelRef::new(&self.provider, &self.model, self.endpoint.clone())
    }

    /// 攻撃文字列を `AttackRef` 群へ解決する（§5.6）．
    ///
    /// `["default"]` は `pil_attacks::default_variants`．個別指定は
    /// `identity` / `base64` / `leetspeak` / `translate:<lang>` / `roleplay:<template>` /
    /// `refusal_suppression`．
    pub fn resolve_attacks(&self) -> Result<Vec<AttackRef>> {
        if self.attacks.len() == 1 && self.attacks[0] == "default" {
            return Ok(default_variants());
        }
        let mut out = Vec::with_capacity(self.attacks.len());
        for spec in &self.attacks {
            let (head, arg) = match spec.split_once(':') {
                Some((h, a)) => (h, Some(a)),
                None => (spec.as_str(), None),
            };
            let attack = match (head, arg) {
                ("identity", None) => identity(),
                ("base64", None) => base64(),
                ("leetspeak", None) => leetspeak(),
                ("refusal_suppression", None) => refusal_suppression(),
                ("translate", Some(lang)) => translate(lang),
                ("roleplay", Some(tpl)) => roleplay(tpl),
                ("default", _) => bail!("`default` は単独指定でのみ有効です"),
                _ => bail!("未知の攻撃指定 `{spec}`"),
            };
            out.push(attack);
        }
        Ok(out)
    }

    /// 測定器指定を検査し正規化名を返す（構築は [`resolve_instruments`]）．
    fn resolve_instrument_specs(&self) -> Result<Vec<String>> {
        if self.instruments.is_empty() {
            bail!("instruments が空です（最低 1 つ必要）");
        }
        for i in &self.instruments {
            match i.as_str() {
                "refusal_match" | "harmbench_cls" | "rubric_v1" | "rubric_v2" => {}
                other => bail!(
                    "未知の測定器 `{other}`（有効: refusal_match / harmbench_cls / rubric_v1 / rubric_v2）"
                ),
            }
        }
        Ok(self.instruments.clone())
    }

    /// 測定器を実体化する（canned judge を配線，§8.2）．
    ///
    /// LLM 生成判定器（cls / rubric）は Phase 1 では [`crate::judges`] の canned judge を使う
    /// （network-free）．judge モデル ID は監査用に `ModelRef` として刻む．
    pub fn resolve_instruments(&self) -> Result<Vec<BoxedInstrument>> {
        let specs = self.resolve_instrument_specs()?;
        let judge_model = ModelRef::new("canned", "canned-judge", None);
        let mut out: Vec<BoxedInstrument> = Vec::with_capacity(specs.len());
        for spec in specs {
            let boxed: BoxedInstrument = match spec.as_str() {
                "refusal_match" => Box::new(RefusalMatch::gcg()),
                "harmbench_cls" => Box::new(HarmBenchCls::new(
                    canned_cls_judge as CannedJudge,
                    judge_model.clone(),
                )),
                "rubric_v1" => Box::new(Rubric::v1(
                    canned_rubric_judge as CannedJudge,
                    judge_model.clone(),
                )),
                "rubric_v2" => Box::new(Rubric::v2(
                    canned_rubric_judge as CannedJudge,
                    judge_model.clone(),
                )),
                other => return Err(anyhow!("未知の測定器 `{other}`")),
            };
            out.push(boxed);
        }
        Ok(out)
    }

    /// 対象ベンチを読み込み，件数上限を適用した Case 群を返す（§7.3，native 直読）．
    ///
    /// 戻り値はベンチ名ごとに分けた `(bench_name, Vec<Case>)`（重複検出や監査で使う）．
    pub fn load_cases(&self, repo_root: impl AsRef<Path>) -> Result<Vec<(String, Vec<Case>)>> {
        let root = repo_root.as_ref();
        let mut out = Vec::new();
        for b in &self.benches {
            let mut cases = load_bench(&b.name, root)?;
            if let Some(limit) = b.limit {
                cases.truncate(limit);
            }
            out.push((b.name.clone(), cases));
        }
        Ok(out)
    }
}

/// 名前でベンチ loader を呼ぶ（native 直読，§7.3）．
pub fn load_bench(name: &str, repo_root: &Path) -> Result<Vec<Case>> {
    let cases = match name {
        "advbench" => pil_bench_advbench::load(repo_root)?,
        "harmbench_text" => pil_bench_harmbench::load_text_all(repo_root)?,
        "harmbench_multimodal" => pil_bench_harmbench::load_multimodal(repo_root)?,
        "strongreject_full" => pil_bench_strongreject::load_full(repo_root)?,
        "strongreject_small" => pil_bench_strongreject::load_small(repo_root)?,
        "jbb_harmful" => pil_bench_jbb::load_harmful(repo_root)?,
        "jbb_benign" => pil_bench_jbb::load_benign(repo_root)?,
        other => bail!("未知のベンチ `{other}`"),
    };
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMOKE_TOML: &str = r#"
name = "test-smoke"
description = "unit test suite"
seed = 42
provider = "mock"
model = "mock-1"
attempts = 3
concurrency = 2
temperature = 0.7
attacks = ["identity", "base64", "translate:zu"]
instruments = ["refusal_match", "harmbench_cls", "rubric_v1"]

[[benches]]
name = "advbench"
limit = 4

[[benches]]
name = "jbb_benign"
limit = 4
"#;

    #[test]
    fn parses_and_resolves_suite() {
        let suite = Suite::from_toml_str(SMOKE_TOML).expect("parse");
        assert_eq!(suite.name, "test-smoke");
        assert_eq!(suite.seed, 42);
        assert_eq!(suite.attempts, 3);
        assert_eq!(suite.benches.len(), 2);

        let attacks = suite.resolve_attacks().expect("attacks");
        assert_eq!(attacks.len(), 3);

        let instruments = suite.resolve_instruments().expect("instruments");
        assert_eq!(instruments.len(), 3);
        // 実体化した測定器の同一性が期待どおり（cls / rubric v1 / refusal-match）
        let names: Vec<String> = instruments.iter().map(|i| i.reference().name).collect();
        assert!(names.contains(&"advbench-refusal-match".to_string()));
        assert!(names.contains(&"harmbench-cls".to_string()));
        assert!(names.contains(&"strongreject-rubric".to_string()));
    }

    #[test]
    fn default_attacks_expands_to_variants() {
        let toml = r#"
name = "d"
[[benches]]
name = "advbench"
"#;
        let suite = Suite::from_toml_str(toml).expect("parse");
        assert_eq!(suite.attacks, vec!["default".to_string()]);
        let attacks = suite.resolve_attacks().expect("attacks");
        assert_eq!(attacks.len(), default_variants().len());
    }

    #[test]
    fn rejects_unknown_bench() {
        let toml = r#"
name = "bad"
[[benches]]
name = "nope"
"#;
        assert!(Suite::from_toml_str(toml).is_err());
    }

    #[test]
    fn rejects_empty_benches() {
        let toml = r#"name = "empty""#;
        assert!(Suite::from_toml_str(toml).is_err());
    }
}
