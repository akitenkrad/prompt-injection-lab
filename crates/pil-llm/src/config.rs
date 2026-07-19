//! `LlmConfig` / `CallMetadata` と seed 規約（DESIGN §6.1 / §11.4）．
//!
//! `socsim-llm` の `LlmConfig`（temperature / seed / max_tokens / system）と
//! `CallMetadata`（model / endpoint / temperature / seed / cache_hit）を踏襲するが，
//! 多試行 ASR（§6.2）と両立させるため seed は `seed = f(attempt)` の規約で導出する．

use serde::{Deserialize, Serialize};

/// 1 生成の設定（DESIGN §6.1）．
///
/// `seed` は **基底 seed** であり，実送信 seed は `attempt` から `seed_for_attempt` で導出する
/// （§11.4）．基底 seed を固定したまま attempt ごとに独立サンプルを引くための規約である．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// サンプリング温度．多試行 ASR は温度 > 0 の独立サンプルを要する（§11.4）
    pub temperature: f32,
    /// 基底 seed．実送信 seed は `seed_for_attempt(seed, attempt)`
    pub seed: u64,
    /// 生成上限トークン数（プロバイダの `num_predict` 等）
    pub max_tokens: Option<u32>,
    /// system プロンプト
    pub system: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            temperature: 0.0,
            seed: 0,
            max_tokens: None,
            system: None,
        }
    }
}

impl LlmConfig {
    /// この設定の基底 seed と `attempt` から，実際に送信する seed を導出する（§11.4）．
    pub fn effective_seed(&self, attempt: u32) -> u64 {
        seed_for_attempt(self.seed, attempt)
    }
}

/// 多試行 seed 規約（DESIGN §11.4）: `seed = base_seed.wrapping_add(attempt)`．
///
/// **根拠**: 多試行 ASR（Anthropic 式 1 / 10 / 100 回開示，§6.2）は温度 > 0 の
/// **独立サンプル**を要する一方で，実験全体の**再現性**は seed 固定に依存する．
/// この 2 要件は「基底 seed を固定しつつ attempt ごとに異なる seed を配る」ことで両立する：
///
/// - **再現性**: `(base_seed, attempt)` が同じなら常に同じ送信 seed → 同じ生成が再現できる．
///   キャッシュキー（§6.2）も同一になり，中断再開（§11.3）で二重生成しない．
/// - **独立性**: attempt が違えば送信 seed が違う → 各試行が別サンプルになり，
///   同一プロンプトの 100 回試行が 1 件に潰れない（`hash(prompt+model)` 型キャッシュの破綻を回避）．
///
/// `wrapping_add` は u64 全域を単調に写す単純規約で，attempt 1..=100 程度では
/// 衝突も桁上がりの飽和も起きない（可読性と決定性を優先し，撹拌はしない）．
#[inline]
pub fn seed_for_attempt(base_seed: u64, attempt: u32) -> u64 {
    base_seed.wrapping_add(attempt as u64)
}

/// 何と話したかの記録（DESIGN §6.1）．全呼び出しで残し，監査・再現の根拠にする．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallMetadata {
    /// 呼び出したモデル（`ModelRef.model` 相当）
    pub model: String,
    /// 呼び出し先エンドポイント（モック時は None）
    pub endpoint: Option<String>,
    /// 実際に用いた温度
    pub temperature: f32,
    /// 実際に送信した seed（`seed_for_attempt` 適用後）
    pub seed: u64,
    /// この応答がキャッシュ由来か（§6.1）
    pub cache_hit: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_reproducible_and_distinct_per_attempt() {
        // 再現性: 同じ (base, attempt) → 同じ seed
        assert_eq!(seed_for_attempt(1000, 7), seed_for_attempt(1000, 7));
        // 独立性: attempt が違えば seed が違う
        assert_ne!(seed_for_attempt(1000, 1), seed_for_attempt(1000, 2));
        // 規約どおり base + attempt
        assert_eq!(seed_for_attempt(1000, 3), 1003);
    }

    #[test]
    fn effective_seed_uses_convention() {
        let cfg = LlmConfig {
            seed: 42,
            ..Default::default()
        };
        assert_eq!(cfg.effective_seed(0), 42);
        assert_eq!(cfg.effective_seed(10), 52);
    }
}
