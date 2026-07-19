//! `Case` / `EnvKind` / `CaseId` / `ContentKey` と正規化（DESIGN §5.2 / §3.4 / §3.5）．
//!
//! `CaseId` は identity（source を含む），`ContentKey` は dedup（source を含めない）．
//! 両者を分けるのが §3.3（出自は当てにならない）/ §3.4（ベンチは互いに独立でない）の帰結．

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hashing::{framed_hash, push_opt_str};
use crate::source::SourceRef;

const CASE_ID_DOMAIN: &str = "pil.caseid.v1";
const CONTENT_KEY_DOMAIN: &str = "pil.contentkey.v1";

/// 環境種別（DESIGN §5.2）．スコア比較の可否を決める科学的性質．第一級メタデータ．
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EnvKind {
    /// LLM 単体・静的プロンプト（Phase 1 の全ベンチ）
    StaticPrompt,
    /// LM emulated 環境（ToolEmu 等）
    Emulated,
    /// 実行可能な実環境（AgentCanary 等）
    RealExecutable,
}

/// Case の identity（DESIGN §5.2）．
///
/// `CaseId = blake3(canonical(prompt) ‖ canonical(context) ‖ SourceRef)` の完全ハッシュ．
/// `SourceRef` を含めるため，同一テキストでも出自が違えば別 `CaseId`（§3.3）．
/// 表示は先頭 16 hex 桁に切り詰めるが，同一性判定は完全値で行う．
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CaseId(String);

/// dedup 用の内容キー（DESIGN §5.2 / §3.4）．
///
/// `ContentKey = blake3(normalize(prompt) ‖ normalize(context))`．出自を含めないため，
/// 別リポジトリの同一テキストが同一キーになる．重複の検出・報告にのみ使い，Case は統合しない．
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContentKey(String);

impl CaseId {
    /// `(prompt, context, source)` から決定論的に導出する．
    pub fn derive(prompt: &str, context: Option<&str>, source: &SourceRef) -> Self {
        let row_le = source.row.to_le_bytes();
        let mut fields: Vec<&[u8]> = Vec::with_capacity(8);
        // canonical(prompt): 恒等（byte-identical な prompt を同一とみなす）
        fields.push(prompt.as_bytes());
        // canonical(context): None / Some を presence フラグで区別
        push_opt_str(&mut fields, context);
        // SourceRef
        fields.push(source.upstream.as_bytes());
        fields.push(source.commit.as_bytes());
        fields.push(source.path.as_bytes());
        fields.push(&row_le);
        Self(framed_hash(CASE_ID_DOMAIN, &fields).to_hex().to_string())
    }

    /// 完全な 64 hex 桁の識別子．
    pub fn full(&self) -> &str {
        &self.0
    }

    /// 表示用の先頭 16 hex 桁（DESIGN §5.2）．
    pub fn short(&self) -> &str {
        &self.0[..16]
    }
}

impl std::fmt::Display for CaseId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.short())
    }
}

impl ContentKey {
    /// `(prompt, context)` から決定論的に導出する（source は含めない）．
    pub fn derive(prompt: &str, context: Option<&str>) -> Self {
        let norm_prompt = normalize(prompt);
        let norm_context = context.map(normalize);
        let mut fields: Vec<&[u8]> = Vec::with_capacity(3);
        fields.push(norm_prompt.as_bytes());
        push_opt_str(&mut fields, norm_context.as_deref());
        Self(
            framed_hash(CONTENT_KEY_DOMAIN, &fields)
                .to_hex()
                .to_string(),
        )
    }

    pub fn full(&self) -> &str {
        &self.0
    }

    pub fn short(&self) -> &str {
        &self.0[..16]
    }
}

impl std::fmt::Display for ContentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.short())
    }
}

/// 正規化規則（DESIGN §3.5）: 小文字化・空白正規化（連続空白を単一スペースに畳む）・末尾ピリオド除去．
///
/// この正規化は `ContentKey`（dedup）専用．§3.5 の「正規化しても潰さない」を守るため，
/// 正規化結果で `Case` を統合してはならない（重複の検出・報告にのみ使う）．
pub fn normalize(s: &str) -> String {
    // 空白正規化 + 小文字化
    let collapsed: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    // 末尾ピリオドと，それに伴い露出する末尾空白を除去
    collapsed
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string()
}

/// 単一の設問（DESIGN §5.2）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Case {
    /// `(prompt, context, source)` から決定論的に導出（§3.5 より context を含める）
    pub id: CaseId,
    pub source: SourceRef,
    /// 攻撃プロンプト本体
    pub prompt: String,
    /// 肯定応答の接頭辞（`"Sure, here is ..."`）．
    /// AdvBench 原典・JBB は `Some` / HarmBench 再梱包は `None`（§3.2）
    pub target: Option<String>,
    /// HarmBench contextual（100 件）用．§3.5 より `CaseId` の導出に含める
    pub context: Option<String>,
    /// 第一級メタデータ．adapter 種別とは別（§4.2）
    pub env_kind: EnvKind,
    /// 良性設問か（JBB benign 100 件）．過剰拒否の測定に使う
    pub benign: bool,
    /// `semantic_category`, `source_claim` 等．同一性の根拠には使わない（§3.3）
    pub labels: BTreeMap<String, String>,
}

impl Case {
    /// 素の材料から `Case` を組み立て，`id` を導出する．
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: SourceRef,
        prompt: impl Into<String>,
        target: Option<String>,
        context: Option<String>,
        env_kind: EnvKind,
        benign: bool,
        labels: BTreeMap<String, String>,
    ) -> Self {
        let prompt = prompt.into();
        let id = CaseId::derive(&prompt, context.as_deref(), &source);
        Self {
            id,
            source,
            prompt,
            target,
            context,
            env_kind,
            benign,
            labels,
        }
    }

    /// この Case の dedup キー（source を含めない）．
    pub fn content_key(&self) -> ContentKey {
        ContentKey::derive(&self.prompt, self.context.as_deref())
    }
}
