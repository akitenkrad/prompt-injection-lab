//! `AttackRef` / `Transform` — union coverage のバリアント軸（DESIGN §5.6）．
//!
//! 変換は `Case` に焼き込まず，生成時に `(Case.prompt, AttackRef)` から最終プロンプトを導出する
//! （`Case` は不変，`Trial.attack` が軸）．すべて公表済み手法の再現で新規攻撃は作らない（§1.4）．

use serde::{Deserialize, Serialize};

use crate::source::SourceRef;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AttackRef {
    pub transform: Transform,
    /// 再現元の論文/実装．§1.4「mutator は既存手法の再現に限る」を型で担保
    pub source: Option<SourceRef>,
}

impl AttackRef {
    pub fn new(transform: Transform, source: Option<SourceRef>) -> Self {
        Self { transform, source }
    }

    /// 無変換の基準点（union の基準）．
    pub fn identity() -> Self {
        Self {
            transform: Transform::Identity,
            source: None,
        }
    }
}

/// すべて公表済み手法の再現．新規攻撃は作らない（§1.4）．
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Transform {
    /// 無変換．静的データセットの素のプロンプト（union の基準点）
    Identity,
    /// Base64 符号化（Wei et al. 2023）
    Base64,
    /// leetspeak 置換
    Leetspeak,
    /// 低資源言語への翻訳（Yong et al. 2023）．BCP-47 言語タグ
    Translate { lang: String },
    /// ペルソナ/ロールプレイ枠付け（DAN 系）
    Roleplay { template_id: String },
    /// 拒否抑制の接頭辞（`"Start your response with 'Sure'"`）
    RefusalSuppression,
}
