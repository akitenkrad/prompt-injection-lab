//! ベンチマーク横断の重複検出（DESIGN §3.4 / §5.2）．
//!
//! §3.4 の主張：**ベンチマークは互いに独立ではない**．JBB は AdvBench / HarmBench から独立
//! しておらず，「3 つのベンチが一致した」と言うとき一部は同じ設問を数えているだけになり得る．
//!
//! 重複検出には provenance（`SourceRef` / `CaseId`，source を含む識別子）は使えない —
//! それらは**別リポジトリの同一テキストを意図的に別物として扱う**からである（§3.4 / §5.2）．
//! そこで正規化テキストの内容フィンガープリント [`pil_core::ContentKey`]（= `Case::content_key()`）
//! を第 2 のキーとして使い，集合間の交差を数える．`ContentKey` は**重複の検出・報告にのみ**使い，
//! Case は統合しない（§3.5 の「正規化しても潰さない」制約）．
//!
//! 本ライブラリは loader に依存しない：入力は名前つきの Case 集合（`&[(name, &[Case])]`）である．
//! 実データの読み込みはテスト側（`pil-bench-*` を dev-dependency に）で行う．

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use pil_core::{Case, ContentKey};

/// 本 crate が重複検出に用いる方式のラベル（§3.4 の「完全一致」との差異を明示する）．
///
/// §3.4 の重複件数は**完全一致（byte-identical）**で測られたが，`ContentKey` は正規化
/// （小文字化・空白正規化・末尾ピリオド除去，§3.5）を経たキーである．レポートには常にこの
/// 方式ラベルを刻み，どちらの尺度で数えたのかを取り違えないようにする．
pub const DUPLICATE_METHOD: &str = "content-key/normalized (§3.5)";

/// 2 集合間の内容重複（DESIGN §3.4）．
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairwiseOverlap {
    /// 集合 A の名前．
    pub a: String,
    /// 集合 B の名前．
    pub b: String,
    /// A・B の双方に現れた `ContentKey` の数（一意キーで数える）．
    pub shared: usize,
}

/// ベンチ横断の重複レポート（DESIGN §3.4）．
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateReport {
    /// 重複を数えた方式（[`DUPLICATE_METHOD`]）．§3.4 の「完全一致」との差異を明示する．
    pub method: String,
    /// 集合名 → その集合に含まれる一意 `ContentKey` 数（集合内の重複は畳んだ後）．
    pub set_sizes: Vec<(String, usize)>,
    /// 全ペアの内容重複（入力順の上三角）．
    pub pairwise: Vec<PairwiseOverlap>,
}

impl DuplicateReport {
    /// 名前で対を引く（順不同）．
    pub fn overlap(&self, a: &str, b: &str) -> Option<usize> {
        self.pairwise
            .iter()
            .find(|p| (p.a == a && p.b == b) || (p.a == b && p.b == a))
            .map(|p| p.shared)
    }

    /// 人間可読レポート（§3.4 の非独立性を明示的に述べる）．
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ベンチ横断の内容重複（方式: {}）\n", self.method));
        for (name, n) in &self.set_sizes {
            out.push_str(&format!("  {name}: 一意 ContentKey {n} 件\n"));
        }
        out.push_str("  ── ペア重複 ──\n");
        let mut any_overlap = false;
        for p in &self.pairwise {
            if p.shared > 0 {
                any_overlap = true;
            }
            out.push_str(&format!("  {} ∩ {} = {}\n", p.a, p.b, p.shared));
        }
        // §3.4: 重複があれば「互いに独立でない」ことを自動で述べる
        if any_overlap {
            out.push_str(
                "  → ベンチマークは互いに独立ではない（§3.4）．\
                 「複数ベンチが一致した」は一部同一設問を重複計上し得る．\n",
            );
        } else {
            out.push_str("  → この対では内容重複は検出されなかった（§3.4）．\n");
        }
        out
    }
}

/// 集合内の一意な `ContentKey` 集合を作る（§5.2：dedup キー）．
fn content_keys(cases: &[Case]) -> BTreeSet<ContentKey> {
    cases.iter().map(|c| c.content_key()).collect()
}

/// 名前つき Case 集合群から，ベンチ横断の内容重複を計算する（DESIGN §3.4）．
///
/// 各集合を一意 `ContentKey` 集合に畳み，全ペアの交差サイズを数える．入力順の上三角を返す．
/// loader には依存しない — Case 集合を入力に取るだけである．
pub fn detect_duplicates(sets: &[(&str, &[Case])]) -> DuplicateReport {
    let keyed: Vec<(String, BTreeSet<ContentKey>)> = sets
        .iter()
        .map(|(name, cases)| ((*name).to_string(), content_keys(cases)))
        .collect();

    let set_sizes = keyed
        .iter()
        .map(|(name, keys)| (name.clone(), keys.len()))
        .collect();

    let mut pairwise = Vec::new();
    for i in 0..keyed.len() {
        for j in (i + 1)..keyed.len() {
            let shared = keyed[i].1.intersection(&keyed[j].1).count();
            pairwise.push(PairwiseOverlap {
                a: keyed[i].0.clone(),
                b: keyed[j].0.clone(),
                shared,
            });
        }
    }

    DuplicateReport {
        method: DUPLICATE_METHOD.to_string(),
        set_sizes,
        pairwise,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use pil_core::{Case, EnvKind, SourceRef};

    /// source を替えても同一テキストは同一 `ContentKey`（§3.4 の重複検出が機能する前提）．
    fn case(prompt: &str, upstream: &str, row: usize) -> Case {
        Case::new(
            SourceRef::new(upstream, "commit", "path", row),
            prompt,
            None,
            None,
            EnvKind::StaticPrompt,
            false,
            BTreeMap::new(),
        )
    }

    #[test]
    fn detects_shared_content_across_sets_ignoring_source() {
        // A と B は "how to X" を共有（source は別），"only in A" は A だけ．
        let a = vec![case("how to X", "repoA", 1), case("only in A", "repoA", 2)];
        let b = vec![
            case("How to X.", "repoB", 9), // 正規化で "how to x" に一致
            case("unique B", "repoB", 10),
        ];
        let rep = detect_duplicates(&[("A", &a), ("B", &b)]);
        assert_eq!(rep.overlap("A", "B"), Some(1));
        assert_eq!(rep.overlap("B", "A"), Some(1)); // 順不同
        assert_eq!(rep.set_sizes, vec![("A".into(), 2), ("B".into(), 2)]);
        assert!(rep.render().contains("独立ではない"));
    }

    #[test]
    fn disjoint_sets_report_zero_and_note_independence() {
        let a = vec![case("alpha", "r", 1)];
        let b = vec![case("beta", "r", 2)];
        let rep = detect_duplicates(&[("A", &a), ("B", &b)]);
        assert_eq!(rep.overlap("A", "B"), Some(0));
        assert!(rep.render().contains("重複は検出されなかった"));
    }

    #[test]
    fn three_sets_produce_three_pairs() {
        let a = vec![case("x", "r", 1)];
        let b = vec![case("x", "s", 2)];
        let c = vec![case("y", "t", 3)];
        let rep = detect_duplicates(&[("A", &a), ("B", &b), ("C", &c)]);
        assert_eq!(rep.pairwise.len(), 3);
        assert_eq!(rep.overlap("A", "B"), Some(1));
        assert_eq!(rep.overlap("A", "C"), Some(0));
        assert_eq!(rep.overlap("B", "C"), Some(0));
    }

    #[test]
    fn serde_roundtrip() {
        let a = vec![case("x", "r", 1)];
        let b = vec![case("x", "s", 2)];
        let rep = detect_duplicates(&[("A", &a), ("B", &b)]);
        let json = serde_json::to_string(&rep).unwrap();
        let back: DuplicateReport = serde_json::from_str(&json).unwrap();
        assert_eq!(rep, back);
    }
}
