//! `pil-llm` — プロバイダ抽象（DESIGN §6）．
//!
//! `socsim-llm` に依存しない独立実装だが，`LlmConfig` / `CallMetadata` / `CachingClient` /
//! feature gate によるネットワーク非依存化という設計は踏襲する（§6.1）．
//!
//! - **既定ビルドは network-free**（§6.1）: HTTP を要するバックエンドは cargo feature で gate し，
//!   既定では reqwest を一切参照しない．テストは `MockProvider` で完結する．
//! - **キャッシュキー**は `hash(rendered_prompt + model + params + attempt + seed)`（§6.2）．
//!   `socsim-llm` の `hash(prompt + model)` をそのまま流用すると多試行 ASR が潰れる（§6.2）ため，
//!   `attempt` と `seed` をキーに含めるのが本 crate の要点である．
//! - **seed 規約**は `seed = base_seed.wrapping_add(attempt)`（§11.4，[`seed_for_attempt`]）．
//! - **logprobs**: `top_logprobs` を返せる API を型で定義する（§6.3．実配線は Ollama native 経路）．
//! - **object-safe** な [`LlmProvider`] トレイトで，`pil-runner` が `dyn LlmProvider` を注入できる（§4.1）．

pub mod backends;
mod cache;
mod config;
mod error;
mod mock;
mod provider;
pub mod version;

pub use cache::{cache_key, CacheAudit, CacheEntry, CachingClient};
pub use config::{seed_for_attempt, CallMetadata, LlmConfig};
pub use error::LlmError;
pub use mock::MockProvider;
pub use provider::{
    BoxFuture, GenerateOutput, GenerateRequest, LlmProvider, TokenLogprobs, ToolCall, ToolSpec,
    TopLogprob,
};
pub use version::{meets_minimum, min_version_string, parse_semver, OLLAMA_MIN_VERSION};

// `pil-core` の応答型を再輸出し，利用側が別途依存せずに扱えるようにする．
pub use pil_core::{FinishReason, ModelRef, Response};
