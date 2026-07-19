//! provenance — 同一性の単位（DESIGN §5.1）．
//!
//! §3.3 の実測（自己申告の出自が当てにならない）に基づき，同一性はベンチマーク名ではなく
//! `(upstream, commit, path, row)` で決める．

use serde::{Deserialize, Serialize};

/// 上流のどこから来たか．`Case` と `Instrument` の同一性の根拠．
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceRef {
    /// 例: `"centerforaisafety/HarmBench"`
    pub upstream: String,
    /// 固定 SHA（フル）．例: `"8e1604d1171fe8a48d8febecd22f600e462bdcdd"`
    pub commit: String,
    /// submodule 内の相対パス．例: `"data/behavior_datasets/harmbench_behaviors_text_all.csv"`
    pub path: String,
    /// CSV のデータ行番号（ヘッダを除き 0 始まり）
    pub row: usize,
}

impl SourceRef {
    pub fn new(
        upstream: impl Into<String>,
        commit: impl Into<String>,
        path: impl Into<String>,
        row: usize,
    ) -> Self {
        Self {
            upstream: upstream.into(),
            commit: commit.into(),
            path: path.into(),
            row,
        }
    }
}
