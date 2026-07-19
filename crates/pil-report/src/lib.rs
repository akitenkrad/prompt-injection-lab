//! `pil-report` — 提示・二値化・重複検出の層（DESIGN §5.3 / §8.1 / §3.4 / §8.4）．
//!
//! 本 crate は **純粋な提示／導出層**である．LLM もネットワークも呼ばず，上流
//! （`pil-metrics::aggregate` / `pil-metrics::reliability`，および `pil-core` の型）が
//! 産んだデータを入力に取り，人間可読・機械可読の両形式へ整形する．
//!
//! 分野の数字が信用できなくなった原因（§8.1）— 三値を黙って二値へ潰す，信頼区間を出さない，
//! ベンチ間の重複を数えないまま「3 ベンチが一致した」と言う — に対し，本層は次を型で担保する：
//!
//! - **二値化は明示操作**（[`binarize`]）．潰した `Undecidable` 件数を必ず併記する（§5.3）．
//! - **ASR の提示は常に信頼区間 + undecidable 件数つき**（[`present`]，§8.1）．
//! - **ベンチ横断の重複検出**（[`duplicates`]）は `ContentKey` を第 2 のキーとして使い，
//!   §3.4 の非独立性（JBB ⊄ AdvBench/HarmBench）を自動レポートする．
//! - **judge 信頼性レポートの整形**（[`reliability_fmt`]）で §3.1 表を再現する（§8.4）．

pub mod binarize;
pub mod duplicates;
pub mod present;
pub mod reliability_fmt;

pub use binarize::{binarize, BinarizationPolicy, BinarizedCounts};
pub use duplicates::{detect_duplicates, DuplicateReport, PairwiseOverlap, DUPLICATE_METHOD};
pub use present::AsrPresentation;
pub use reliability_fmt::format_reliability;
