//! `pil-bench-harmbench` — HarmBench behavior datasets の native ローダ（DESIGN §7.3 / §9.1）．
//!
//! HarmBench は **ファイルごとにスキーマが違う**（§9.1）ため，列は位置ではなく **ヘッダ名で引く**．
//! これにより multimodal の列順違い（9 列）も自然に吸収できる．`ContextString` は埋め込み改行・引用符を
//! 含む真の複数行 RFC4180 quoted field なので，`csv` crate の既定 quoting に委ねる（素朴な行分割は壊れる）．
//! `Tags` は `", "`（カンマ+空白）区切りの文字列であり JSON 配列ではない（上流は `split(", ")`）．
//!
//! §3.2: HarmBench 再梱包の AdvBench は `target` 列を持たない．よって全 Case で `target = None`．
//! contextual（`context` タグ・非空 `ContextString`）は `Case.context = Some(..)`．
//! copyright（`hash_check` タグ）は MinHash 照合対象（M3）で，ここでは `labels` にタグを残すのみ．

use std::collections::BTreeMap;
use std::path::Path;

use pil_core::{Case, EnvKind, SourceRef};

/// 上流リポジトリ識別子（DESIGN §7.1）．
pub const UPSTREAM: &str = "centerforaisafety/HarmBench";
/// 固定 SHA（DESIGN §7.1）．submodule pin と一致する．
pub const COMMIT: &str = "8e1604d1171fe8a48d8febecd22f600e462bdcdd";
/// text behaviors（400 件）の相対パス．Phase 1 の「HarmBench 400」はこのファイル（§10）．
pub const TEXT_ALL_PATH: &str = "data/behavior_datasets/harmbench_behaviors_text_all.csv";
/// multimodal behaviors（110 件・9 列・列順違い）の相対パス（§9.1）．
pub const MULTIMODAL_PATH: &str = "data/behavior_datasets/harmbench_behaviors_multimodal_all.csv";
/// repo root からの submodule 相対プレフィックス．
const SUBMODULE_PREFIX: &str = "third_party/HarmBench";

/// `Tags` 中で contextual を示すトークン（§9.1 / §8.2）．
pub const TAG_CONTEXT: &str = "context";
/// `Tags` 中で copyright（MinHash 照合）を示すトークン（§8.2）．
pub const TAG_HASH_CHECK: &str = "hash_check";
/// `Tags` の区切り（§9.1: カンマ + 空白）．
const TAG_SEPARATOR: &str = ", ";

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

/// `Tags` フィールドを `", "` で分割する（§9.1）．空要素は捨てる．
pub fn split_tags(tags: &str) -> Vec<String> {
    tags.split(TAG_SEPARATOR)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// ヘッダから列名 index を引くアクセサ（ファイル別スキーマを名前解決で吸収）．
struct HeaderMap {
    positions: BTreeMap<String, usize>,
    path: String,
}

impl HeaderMap {
    fn new(headers: &csv::StringRecord, path: String) -> Self {
        let positions = headers
            .iter()
            .enumerate()
            .map(|(i, h)| (h.to_string(), i))
            .collect();
        Self { positions, path }
    }

    /// 必須列を引く（無ければ `MissingColumn`）．
    fn require(
        &self,
        record: &csv::StringRecord,
        name: &'static str,
    ) -> Result<String, LoaderError> {
        let idx = self
            .positions
            .get(name)
            .copied()
            .ok_or_else(|| LoaderError::MissingColumn {
                column: name,
                path: self.path.clone(),
            })?;
        Ok(record.get(idx).unwrap_or_default().to_string())
    }

    /// 任意列を引く（無ければ `None`．ファイル別スキーマ差の吸収に使う）．
    fn optional(&self, record: &csv::StringRecord, name: &str) -> Option<String> {
        self.positions
            .get(name)
            .and_then(|&idx| record.get(idx))
            .map(str::to_string)
    }
}

/// HarmBench text behaviors 400 件を読み込む（Phase 1 の「HarmBench 400」，§10）．
///
/// スキーマ: `Behavior,FunctionalCategory,SemanticCategory,Tags,ContextString,BehaviorID`（§9.1）．
pub fn load_text_all(repo_root: impl AsRef<Path>) -> Result<Vec<Case>, LoaderError> {
    load_file(repo_root, TEXT_ALL_PATH, ContextField::ContextString)
}

/// HarmBench multimodal behaviors 110 件を読み込む（9 列・列順違いの回帰対象，§9.1）．
///
/// スキーマ（列順違い）: `Behavior,BehaviorID,FunctionalCategory,SemanticCategory,
/// ImageFileName,Source,ImageDescription,RedactedImageDescription,Tags`．
/// context は §8.2 に従い `RedactedImageDescription` を用いる．
pub fn load_multimodal(repo_root: impl AsRef<Path>) -> Result<Vec<Case>, LoaderError> {
    load_file(
        repo_root,
        MULTIMODAL_PATH,
        ContextField::RedactedImageDescription,
    )
}

/// どの列を `Case.context` の供給源にするか（§8.2 のテンプレート選択に対応）．
#[derive(Clone, Copy)]
enum ContextField {
    /// text 系: `ContextString`．
    ContextString,
    /// multimodal 系: `RedactedImageDescription`（§8.2）．
    RedactedImageDescription,
}

impl ContextField {
    fn column(self) -> &'static str {
        match self {
            ContextField::ContextString => "ContextString",
            ContextField::RedactedImageDescription => "RedactedImageDescription",
        }
    }
}

/// 汎用ローダ本体．ヘッダ名解決で任意スキーマを吸収する．
fn load_file(
    repo_root: impl AsRef<Path>,
    rel_path: &'static str,
    context_field: ContextField,
) -> Result<Vec<Case>, LoaderError> {
    let path = repo_root.as_ref().join(SUBMODULE_PREFIX).join(rel_path);
    let path_disp = path.display().to_string();

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .from_path(&path)?;

    let headers = reader.headers()?.clone();
    let hm = HeaderMap::new(&headers, path_disp);

    let mut cases = Vec::new();
    for (row, record) in reader.records().enumerate() {
        let record = record?;

        let behavior = hm.require(&record, "Behavior")?;
        let behavior_id = hm.require(&record, "BehaviorID")?;
        let tags_raw = hm.optional(&record, "Tags").unwrap_or_default();
        let tags = split_tags(&tags_raw);

        // context: 該当列が非空なら Some（§3.5 より CaseId 導出に効く）．
        let context = hm
            .optional(&record, context_field.column())
            .filter(|s| !s.trim().is_empty());

        // labels: 同一性の根拠には使わないメタ（§3.3）．ファイル別に存在する列を拾う．
        let mut labels = BTreeMap::new();
        labels.insert("behavior_id".to_string(), behavior_id);
        if !tags_raw.is_empty() {
            labels.insert("tags".to_string(), tags.join(","));
        }
        if let Some(fc) = hm.optional(&record, "FunctionalCategory") {
            labels.insert("functional_category".to_string(), fc);
        }
        // SemanticCategory は text/multimodal に，Category は extra/2_behaviors にある（§9.1）．
        if let Some(sc) = hm.optional(&record, "SemanticCategory") {
            labels.insert("semantic_category".to_string(), sc);
        }
        if let Some(cat) = hm.optional(&record, "Category") {
            labels.insert("category".to_string(), cat);
        }
        if let Some(src) = hm.optional(&record, "Source") {
            labels.insert("source_claim".to_string(), src);
        }

        let source = SourceRef::new(UPSTREAM, COMMIT, rel_path, row);
        cases.push(Case::new(
            source,
            behavior,
            None, // §3.2: HarmBench は target を持たない
            context,
            EnvKind::StaticPrompt,
            false,
            labels,
        ));
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn text_all_loads_exactly_400() {
        let cases = load_text_all(repo_root()).expect("load text_all");
        assert_eq!(
            cases.len(),
            400,
            "HarmBench text behaviors は 400 件（DESIGN §10）"
        );
    }

    #[test]
    fn multimodal_loads_110_with_column_order_swap() {
        // multimodal は BehaviorID が 2 列目（列順違い）．名前解決で正しく引けることを確認（§9.1）．
        let cases = load_multimodal(repo_root()).expect("load multimodal");
        assert_eq!(cases.len(), 110);
        for c in &cases {
            assert!(c.target.is_none());
            assert!(c.labels.contains_key("behavior_id"));
            // Behavior（1 列目）と BehaviorID（2 列目）が取り違えられていないこと．
            assert!(!c.prompt.is_empty());
            assert_ne!(c.prompt, c.labels["behavior_id"]);
        }
    }

    #[test]
    fn target_is_none_everywhere() {
        let cases = load_text_all(repo_root()).unwrap();
        assert!(
            cases.iter().all(|c| c.target.is_none()),
            "§3.2: HarmBench target なし"
        );
    }

    #[test]
    fn tag_counts_match_measured() {
        // §9.1 / 実測: context=100, hash_check=100, book=50, lyrics=50．
        let cases = load_text_all(repo_root()).unwrap();
        let mut context = 0;
        let mut hash_check = 0;
        let mut book = 0;
        let mut lyrics = 0;
        for c in &cases {
            let tags: Vec<&str> = c
                .labels
                .get("tags")
                .map(|t| t.split(',').collect())
                .unwrap_or_default();
            if tags.contains(&TAG_CONTEXT) {
                context += 1;
            }
            if tags.contains(&TAG_HASH_CHECK) {
                hash_check += 1;
            }
            if tags.contains(&"book") {
                book += 1;
            }
            if tags.contains(&"lyrics") {
                lyrics += 1;
            }
        }
        assert_eq!(context, 100, "contextual 100");
        assert_eq!(hash_check, 100, "copyright 100");
        assert_eq!(book, 50);
        assert_eq!(lyrics, 50);
    }

    #[test]
    fn contextual_rows_carry_context() {
        // context タグの行は非空 ContextString を持ち，Case.context = Some になる（100 件）．
        let cases = load_text_all(repo_root()).unwrap();
        let with_context = cases.iter().filter(|c| c.context.is_some()).count();
        assert_eq!(
            with_context, 100,
            "非空 ContextString は 100 件（= contextual）"
        );
        for c in &cases {
            let is_context_tag = c
                .labels
                .get("tags")
                .map(|t| t.split(',').any(|x| x == TAG_CONTEXT))
                .unwrap_or(false);
            assert_eq!(
                is_context_tag,
                c.context.is_some(),
                "context タグ ⇔ Case.context"
            );
        }
    }

    #[test]
    fn embedded_newline_context_parsed_as_single_field() {
        // §9.1 の落とし穴: ContextString は埋め込み改行を含む複数行 quoted field．
        // 素朴な行分割なら壊れるが csv crate の RFC4180 quoting は 1 フィールドに畳む．
        let cases = load_text_all(repo_root()).unwrap();
        let ctx = cases
            .iter()
            .find(|c| {
                c.labels.get("behavior_id").map(String::as_str) == Some("lsd_25_highschool_explain")
            })
            .expect("row with embedded newline");
        let s = ctx.context.as_deref().expect("has context");
        assert!(s.contains('\n'), "埋め込み改行が保持されている");
    }

    #[test]
    fn embedded_quote_context_parsed() {
        // §9.1: ContextString は引用符も含む（RFC4180 の二重引用符エスケープ）．
        let cases = load_text_all(repo_root()).unwrap();
        let ctx = cases
            .iter()
            .find(|c| {
                c.labels.get("behavior_id").map(String::as_str)
                    == Some("email_emma_jackson_malware_persuasion")
            })
            .expect("row with embedded quote");
        let s = ctx.context.as_deref().expect("has context");
        assert!(s.contains('"'), "エスケープされた引用符が復元されている");
    }

    #[test]
    fn split_tags_handles_separator() {
        assert_eq!(
            split_tags("lyrics, hash_check"),
            vec!["lyrics", "hash_check"]
        );
        assert_eq!(split_tags("context"), vec!["context"]);
        assert!(split_tags("").is_empty());
    }

    #[test]
    fn provenance_rows_are_sequential() {
        let cases = load_text_all(repo_root()).unwrap();
        for (i, c) in cases.iter().enumerate() {
            assert_eq!(c.source.row, i);
            assert_eq!(c.source.commit, COMMIT);
            assert_eq!(c.source.path, TEXT_ALL_PATH);
        }
    }
}
