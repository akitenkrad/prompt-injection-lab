//! `pil-shim` — OpenAI 互換ローカルシム（DESIGN §4.1「制御の反転」）．
//!
//! Rust プロセスが OpenAI 互換のローカルエンドポイントを立て，外部（Python）ベンチの `base_url` を
//! そこへ向けさせる．これにより**モデル呼び出しが 2 系統に分裂するのを防ぎ**，全ての生成要求を
//! `pil-llm` の単一経路（`Arc<dyn LlmProvider>`）に集約する（温度・seed・cache・metadata・
//! rate-limit が全経路で揃う，§4.1）．
//!
//! - **既定ビルドは network-free**（§6.1）: OpenAI 型（[`openai`]）と純変換（[`mapping`]）のみを
//!   含み，HTTP サーバ（axum/tokio net）は feature `shim` でのみ導入する．既定では axum を一切
//!   参照しない（bind/listen だけが `shim` の裏に隠れる）．
//! - **純変換は feature 非依存**: OpenAI ⇄ [`pil_llm::GenerateRequest`] / [`pil_llm::GenerateOutput`]
//!   の写像は副作用なしで単体テストできる（§4.1）．
//! - **logprobs**: OpenAI `logprobs` / `top_logprobs` を `GenerateRequest.top_logprobs` に写す（§6.3）．

pub mod mapping;
pub mod openai;

#[cfg(feature = "shim")]
pub mod server;

pub use mapping::{
    error_to_openai, finish_reason_to_openai, model_ref_from_model, render_messages,
    to_chat_completion_response, to_generate_request, SHIM_DEFAULT_ATTEMPT,
};
pub use openai::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChoiceLogprobs,
    ModelList, ModelObject, OpenAiError, OpenAiErrorResponse, TokenLogprobEntry, TopLogprobEntry,
    Usage,
};

#[cfg(feature = "shim")]
pub use server::{router, serve, ShimState};
