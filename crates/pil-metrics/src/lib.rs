//! `pil-metrics` — 測定器・集計・信頼性（DESIGN §8）．
//!
//! crate 境界は一つだが，内部は「1 件ずつ判定する測定器（`instrument`）」「全件から出す集計
//! （`aggregate`）」「判定器自身の信頼性（`reliability`）」に分ける（§8.1）．異なる壊れ方をする
//! ためである．依存の向きは `reliability → instrument`．
//!
//! - M2（本コミット）: `reliability` — network-free の差別化点．§3.1 を回帰テストで再現
//! - M3: `instrument` — 4 種の測定器
//! - M8: `aggregate` — ASR / 信頼区間 / union / 多試行

pub mod aggregate;
pub mod instrument;
pub mod reliability;
