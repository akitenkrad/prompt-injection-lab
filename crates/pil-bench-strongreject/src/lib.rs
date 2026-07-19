//! `pil-bench-strongreject` — StrongREJECT データセットの native ローダ（DESIGN §7.3 / §9.1）．
//!
//! 原典 `alexandrasouly/strongreject`@`f7cad6c` の `strongreject_dataset/` を submodule 直読する．
//! full（313 件）と small（60 件）を独立に読む．**small は full の部分列ではなく順序も違う**（§9.1）ため，
//! 片方を他方から派生させない．スキーマは両者とも `category,source,forbidden_prompt`．
//!
//! §3.2: StrongREJECT は target 列を持たない（拒否文字列マッチではなくルーブリック採点で測る）．よって
//! 全 Case で `target = None`．ルーブリック v1/v2 プロンプト文言の native 再実装は M3 の責務であり，
//! 本 crate は **データセットの読み取りのみ**を担う（§3.7 / IMPLEMENTATION_PLAN M5）．

use std::collections::BTreeMap;
use std::path::Path;

use pil_core::{Case, EnvKind, SourceRef};

/// 上流リポジトリ識別子（DESIGN §7.1）．
pub const UPSTREAM: &str = "alexandrasouly/strongreject";
/// 固定 SHA（DESIGN §7.1）．submodule pin と一致する．
pub const COMMIT: &str = "f7cad6c17e624e21d8df2278e918ae1dddb4cb56";
/// full（313 件）の相対パス．
pub const FULL_PATH: &str = "strongreject_dataset/strongreject_dataset.csv";
/// small（60 件）の相対パス．
pub const SMALL_PATH: &str = "strongreject_dataset/strongreject_small_dataset.csv";
/// repo root からの submodule 相対プレフィックス．
const SUBMODULE_PREFIX: &str = "third_party/strongreject";

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
}

/// StrongREJECT full 313 件を読み込む．
pub fn load_full(repo_root: impl AsRef<Path>) -> Result<Vec<Case>, LoaderError> {
    load_file(repo_root, FULL_PATH)
}

/// StrongREJECT small 60 件を読み込む（full の部分列ではない，§9.1）．
pub fn load_small(repo_root: impl AsRef<Path>) -> Result<Vec<Case>, LoaderError> {
    load_file(repo_root, SMALL_PATH)
}

/// 共通ローダ本体．スキーマ `category,source,forbidden_prompt`．
fn load_file(
    repo_root: impl AsRef<Path>,
    rel_path: &'static str,
) -> Result<Vec<Case>, LoaderError> {
    let path = repo_root.as_ref().join(SUBMODULE_PREFIX).join(rel_path);
    let path_disp = path.display().to_string();

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&path)?;

    let headers = reader.headers()?.clone();
    let cat_idx = column_index(&headers, "category").ok_or_else(|| LoaderError::MissingColumn {
        column: "category",
        path: path_disp.clone(),
    })?;
    let src_idx = column_index(&headers, "source").ok_or_else(|| LoaderError::MissingColumn {
        column: "source",
        path: path_disp.clone(),
    })?;
    let prompt_idx =
        column_index(&headers, "forbidden_prompt").ok_or_else(|| LoaderError::MissingColumn {
            column: "forbidden_prompt",
            path: path_disp.clone(),
        })?;

    let mut cases = Vec::new();
    for (row, record) in reader.records().enumerate() {
        let record = record?;
        let category = record.get(cat_idx).unwrap_or_default().to_string();
        let source_claim = record.get(src_idx).unwrap_or_default().to_string();
        let prompt = record.get(prompt_idx).unwrap_or_default().to_string();

        // labels: 同一性の根拠には使わないメタ（§3.3）．
        let mut labels = BTreeMap::new();
        labels.insert("category".to_string(), category);
        labels.insert("source_claim".to_string(), source_claim);

        let source = SourceRef::new(UPSTREAM, COMMIT, rel_path, row);
        cases.push(Case::new(
            source,
            prompt,
            None, // §3.2: StrongREJECT は target なし
            None, // context なし
            EnvKind::StaticPrompt,
            false,
            labels,
        ));
    }
    Ok(cases)
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
    fn full_loads_exactly_313() {
        let cases = load_full(repo_root()).expect("load full");
        assert_eq!(
            cases.len(),
            313,
            "StrongREJECT full は 313 件（DESIGN §10）"
        );
    }

    #[test]
    fn small_loads_exactly_60() {
        let cases = load_small(repo_root()).expect("load small");
        assert_eq!(cases.len(), 60, "StrongREJECT small は 60 件（DESIGN §10）");
    }

    #[test]
    fn small_is_not_a_prefix_of_full() {
        // §9.1: small は full の部分列でなく順序も違う．先頭 prompt が食い違うことを確認．
        let full = load_full(repo_root()).unwrap();
        let small = load_small(repo_root()).unwrap();
        assert_ne!(
            full[0].prompt, small[0].prompt,
            "small は full の prefix ではない（順序が違う）"
        );
    }

    #[test]
    fn no_target_static_prompt_and_labels() {
        let cases = load_full(repo_root()).unwrap();
        for (i, c) in cases.iter().enumerate() {
            assert!(c.target.is_none(), "§3.2: StrongREJECT target なし");
            assert!(c.context.is_none());
            assert_eq!(c.env_kind, EnvKind::StaticPrompt);
            assert!(!c.benign);
            assert!(c.labels.contains_key("category"));
            assert!(c.labels.contains_key("source_claim"));
            assert_eq!(c.source.row, i);
            assert_eq!(c.source.commit, COMMIT);
            assert_eq!(c.source.path, FULL_PATH);
        }
    }
}
