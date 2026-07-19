//! `Trial` — 1 回の生成に測定器を複数ぶら下げる（DESIGN §5.5）．
//!
//! `measurements: Vec<Measurement>` が本設計の要．1 回の生成に対して v1/v2 ルーブリック・
//! HarmBench 分類器・文字列マッチを同時に当て，測定器間の不一致を追加コストほぼゼロで取る．

use serde::{Deserialize, Serialize};

use crate::attack::AttackRef;
use crate::case::CaseId;
use crate::instrument::Measurement;
use crate::model::{ModelRef, Response};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trial {
    pub case: CaseId,
    /// 多試行 ASR 用（1..=100）．キャッシュキーに含める（§6.2）
    pub attempt: u32,
    pub model: ModelRef,
    /// 攻撃バリアント．union coverage の算出に使う
    pub attack: AttackRef,
    pub response: Response,
    /// 同一応答を複数の測定器で判定する
    pub measurements: Vec<Measurement>,
}
