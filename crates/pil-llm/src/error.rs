//! `pil-llm` のエラー型（DESIGN §6）．
//!
//! プロバイダ層のエラーは，`pil-core` の `Verdict::Undecidable{ ProviderError }`（§5.3）へ
//! 写像できるよう文字列メッセージを保持する．

/// プロバイダ呼び出しで起こり得る失敗．
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// バックエンド未実装（OpenAI / Anthropic / Gemini の Phase 1 骨組み）．
    #[error("provider backend not implemented: {0}")]
    NotImplemented(String),

    /// Ollama のバージョンが要件（§6.3 `>= 0.12.11`）未満．黙って劣化させず明示失敗する．
    #[error("unsupported Ollama version: found {found}, require >= {required}")]
    UnsupportedVersion {
        /// 実際に検出したバージョン文字列
        found: String,
        /// 要求する最小バージョン
        required: String,
    },

    /// バージョン文字列を解釈できなかった（`/api/version` の応答が異常）．
    #[error("could not parse Ollama version string: {0:?}")]
    UnparsableVersion(String),

    /// ネットワーク／トランスポート層の失敗（接続不可・タイムアウト等）．
    #[error("network error: {0}")]
    Network(String),

    /// プロバイダが業務的エラーを返した（HTTP 4xx/5xx・レート制限等）．
    #[error("provider returned an error: {0}")]
    Provider(String),

    /// プロバイダ応答のパースに失敗した．
    #[error("failed to parse provider response: {0}")]
    Parse(String),
}
