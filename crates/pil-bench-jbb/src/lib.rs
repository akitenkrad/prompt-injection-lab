//! `pil-bench-jbb` — JailbreakBench (JBB-Behaviors) の native ローダ（DESIGN §7.3 / §8.4）．
//!
//! HuggingFace の git リポジトリ（`JailbreakBench/JBB-Behaviors`@`886acc3`）を submodule 直読する．
//! 3 ファイルを扱う:
//! - `data/harmful-behaviors.csv`（100 件，`Goal/Target` あり → `Case.target = Some`）
//! - `data/benign-behaviors.csv`（100 件，過剰拒否の測定用．`benign = true`，§10）
//! - `data/judge-comparison.csv`（300 件，人手 3 名 + 多数決 + 分類器 4 種．§3.1 / §8.4 の回帰の正解データ）
//!
//! judge-comparison は `Case` ではなく専用型 [`JudgeComparisonRow`] で返す（`pil-metrics::reliability`
//! が §3.1 の数値を再現するために消費する．本 crate は pil-metrics を編集しない）．

use std::collections::BTreeMap;
use std::path::Path;

use pil_core::{Case, EnvKind, SourceRef};

/// 上流リポジトリ識別子（DESIGN §7.1）．
pub const UPSTREAM: &str = "JailbreakBench/JBB-Behaviors";
/// 固定 SHA（DESIGN §7.1）．submodule pin と一致する．
pub const COMMIT: &str = "886acc352a31533ffbcf4ef22c744658688086fc";
/// harmful behaviors の相対パス．
pub const HARMFUL_PATH: &str = "data/harmful-behaviors.csv";
/// benign behaviors の相対パス．
pub const BENIGN_PATH: &str = "data/benign-behaviors.csv";
/// judge-comparison の相対パス．
pub const JUDGE_COMPARISON_PATH: &str = "data/judge-comparison.csv";
/// repo root からの submodule 相対プレフィックス．
const SUBMODULE_PREFIX: &str = "third_party/JBB-Behaviors";

/// ローダのエラー．
#[derive(Debug, thiserror::Error)]
pub enum LoaderError {
    /// CSV 読み取り／IO 失敗．
    #[error("CSV 読み取りに失敗しました: {0}")]
    Csv(#[from] csv::Error),
    /// 期待する列が存在しない（スキーマ不整合）．
    #[error("列 `{column}` が {path} に見つかりません")]
    MissingColumn {
        /// 見つからなかった列名．
        column: &'static str,
        /// 対象ファイルパス．
        path: String,
    },
    /// 二値ラベル列が `0` / `1` 以外の値を持つ（判定列の破損）．
    #[error("列 `{column}`（{path} 行 {row}）が二値ラベルでない: {value:?}")]
    NonBinaryLabel {
        /// 対象列名．
        column: &'static str,
        /// 対象ファイルパス．
        path: String,
        /// データ行番号（0 始まり）．
        row: usize,
        /// 実際の値．
        value: String,
    },
}

/// JBB harmful behaviors 100 件を読み込む（`benign = false`，`target = Some`）．
///
/// スキーマ: `Index,Goal,Target,Behavior,Category,Source`．
pub fn load_harmful(repo_root: impl AsRef<Path>) -> Result<Vec<Case>, LoaderError> {
    load_behaviors(repo_root, HARMFUL_PATH, false)
}

/// JBB benign behaviors 100 件を読み込む（`benign = true`，過剰拒否の測定用，§10）．
///
/// スキーマは harmful と同一．`Target` 列は良性設問への肯定接頭辞であり `Some` で保持する．
pub fn load_benign(repo_root: impl AsRef<Path>) -> Result<Vec<Case>, LoaderError> {
    load_behaviors(repo_root, BENIGN_PATH, true)
}

/// harmful / benign 共通ローダ．`Index,Goal,Target,Behavior,Category,Source`．
fn load_behaviors(
    repo_root: impl AsRef<Path>,
    rel_path: &'static str,
    benign: bool,
) -> Result<Vec<Case>, LoaderError> {
    let path = repo_root.as_ref().join(SUBMODULE_PREFIX).join(rel_path);
    let path_disp = path.display().to_string();

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&path)?;

    let headers = reader.headers()?.clone();
    let idx = |name: &'static str| -> Result<usize, LoaderError> {
        column_index(&headers, name).ok_or_else(|| LoaderError::MissingColumn {
            column: name,
            path: path_disp.clone(),
        })
    };
    let goal_i = idx("Goal")?;
    let target_i = idx("Target")?;
    let behavior_i = idx("Behavior")?;
    let category_i = idx("Category")?;
    let source_i = idx("Source")?;
    let index_i = idx("Index")?;

    let mut cases = Vec::with_capacity(100);
    for (row, record) in reader.records().enumerate() {
        let record = record?;
        let get = |i: usize| record.get(i).unwrap_or_default().to_string();
        let goal = get(goal_i);
        let target = get(target_i);

        // labels: 自己申告の出自（§3.3）は同一性の根拠にせず，ラベルとしてのみ保持する．
        let mut labels = BTreeMap::new();
        labels.insert("index".to_string(), get(index_i));
        labels.insert("behavior".to_string(), get(behavior_i));
        labels.insert("category".to_string(), get(category_i));
        labels.insert("source_claim".to_string(), get(source_i));

        let source = SourceRef::new(UPSTREAM, COMMIT, rel_path, row);
        cases.push(Case::new(
            source,
            goal,
            Some(target), // JBB は Target 列あり（§3.2）
            None,
            EnvKind::StaticPrompt,
            benign,
            labels,
        ));
    }
    Ok(cases)
}

/// judge-comparison.csv の 1 行（DESIGN §8.4）．
///
/// 人手 3 名の独立ラベル・多数決・分類器 4 種の判定が揃った，ネットワーク不要の回帰用正解データ．
/// 二値ラベル（`0`/`1`）は `bool` に正規化する（`1` = 有害/攻撃成功と判定）．
#[derive(Debug, Clone, PartialEq)]
pub struct JudgeComparisonRow {
    /// `Index` 列（上流の連番）．
    pub index: String,
    /// 攻撃の目標（`goal`）．
    pub goal: String,
    /// 判定対象のプロンプト（`prompt`）．
    pub prompt: String,
    /// 判定対象の応答（`target_response`）．
    pub target_response: String,
    /// 人手アノテータ 1 の判定．
    pub human1: bool,
    /// 人手アノテータ 2 の判定．
    pub human2: bool,
    /// 人手アノテータ 3 の判定．
    pub human3: bool,
    /// 人手多数決（`human_majority`）．真の ASR の基準（§3.1）．
    pub human_majority: bool,
    /// `harmbench_cf` 分類器の判定（FPR 26.8% の当該分類器，§3.1）．
    pub harmbench_cf: bool,
    /// `gpt4_cf` 分類器の判定．
    pub gpt4_cf: bool,
    /// `llamaguard2_cf` 分類器の判定．
    pub llamaguard2_cf: bool,
    /// `llama3_cf` 分類器の判定．
    pub llama3_cf: bool,
}

/// judge-comparison 300 件を読み込む（§8.4 の reliability 回帰フィクスチャ）．
pub fn load_judge_comparison(
    repo_root: impl AsRef<Path>,
) -> Result<Vec<JudgeComparisonRow>, LoaderError> {
    let path = repo_root
        .as_ref()
        .join(SUBMODULE_PREFIX)
        .join(JUDGE_COMPARISON_PATH);
    let path_disp = path.display().to_string();

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&path)?;

    let headers = reader.headers()?.clone();
    let idx = |name: &'static str| -> Result<usize, LoaderError> {
        column_index(&headers, name).ok_or_else(|| LoaderError::MissingColumn {
            column: name,
            path: path_disp.clone(),
        })
    };
    let index_i = idx("Index")?;
    let goal_i = idx("goal")?;
    let prompt_i = idx("prompt")?;
    let resp_i = idx("target_response")?;
    let h1_i = idx("human1")?;
    let h2_i = idx("human2")?;
    let h3_i = idx("human3")?;
    let hm_i = idx("human_majority")?;
    let hb_i = idx("harmbench_cf")?;
    let gpt4_i = idx("gpt4_cf")?;
    let lg2_i = idx("llamaguard2_cf")?;
    let l3_i = idx("llama3_cf")?;

    let mut rows = Vec::with_capacity(300);
    for (row, record) in reader.records().enumerate() {
        let record = record?;
        let text = |i: usize| record.get(i).unwrap_or_default().to_string();
        let flag = |i: usize, col: &'static str| -> Result<bool, LoaderError> {
            parse_binary(record.get(i).unwrap_or_default(), col, &path_disp, row)
        };
        rows.push(JudgeComparisonRow {
            index: text(index_i),
            goal: text(goal_i),
            prompt: text(prompt_i),
            target_response: text(resp_i),
            human1: flag(h1_i, "human1")?,
            human2: flag(h2_i, "human2")?,
            human3: flag(h3_i, "human3")?,
            human_majority: flag(hm_i, "human_majority")?,
            harmbench_cf: flag(hb_i, "harmbench_cf")?,
            gpt4_cf: flag(gpt4_i, "gpt4_cf")?,
            llamaguard2_cf: flag(lg2_i, "llamaguard2_cf")?,
            llama3_cf: flag(l3_i, "llama3_cf")?,
        });
    }
    Ok(rows)
}

/// `0` / `1` を `bool` に正規化する．それ以外は `NonBinaryLabel`．
fn parse_binary(
    value: &str,
    column: &'static str,
    path: &str,
    row: usize,
) -> Result<bool, LoaderError> {
    match value.trim() {
        "0" => Ok(false),
        "1" => Ok(true),
        other => Err(LoaderError::NonBinaryLabel {
            column,
            path: path.to_string(),
            row,
            value: other.to_string(),
        }),
    }
}

/// ヘッダ StringRecord から列名の index を引く．
fn column_index(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers.iter().position(|h| h == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn harmful_loads_100_with_target() {
        let cases = load_harmful(repo_root()).expect("load harmful");
        assert_eq!(cases.len(), 100, "JBB harmful は 100 件（DESIGN §10）");
        for (i, c) in cases.iter().enumerate() {
            assert!(c.target.is_some(), "JBB は Target あり（§3.2）");
            assert!(!c.benign);
            assert_eq!(c.env_kind, EnvKind::StaticPrompt);
            assert_eq!(c.source.row, i);
            assert_eq!(c.source.path, HARMFUL_PATH);
        }
    }

    #[test]
    fn benign_loads_100_flagged_benign() {
        let cases = load_benign(repo_root()).expect("load benign");
        assert_eq!(cases.len(), 100, "JBB benign は 100 件（DESIGN §10）");
        assert!(
            cases.iter().all(|c| c.benign),
            "benign フラグは全件 true（§10）"
        );
    }

    #[test]
    fn judge_comparison_loads_300() {
        let rows = load_judge_comparison(repo_root()).expect("load judge-comparison");
        assert_eq!(rows.len(), 300, "judge-comparison は 300 件（DESIGN §8.4）");
    }

    #[test]
    fn judge_comparison_reproduces_asr_figures() {
        // §3.1 の再現: 真 ASR = human_majority の 1 の割合 = 36.7%（110/300），
        // harmbench_cf の報告 ASR = 49.0%（147/300）．二値パースが正しいことの回帰．
        let rows = load_judge_comparison(repo_root()).unwrap();
        let true_positive = rows.iter().filter(|r| r.human_majority).count();
        let hb_positive = rows.iter().filter(|r| r.harmbench_cf).count();
        assert_eq!(true_positive, 110, "真 ASR 110/300 = 36.7%（§3.1）");
        assert_eq!(hb_positive, 147, "harmbench_cf 147/300 = 49.0%（§3.1）");
    }

    #[test]
    fn source_claim_kept_but_not_identity() {
        // §3.3: 自己申告の出自はラベルとして保持するが同一性の根拠にはしない．
        let cases = load_harmful(repo_root()).unwrap();
        assert!(cases.iter().all(|c| c.labels.contains_key("source_claim")));
    }
}
