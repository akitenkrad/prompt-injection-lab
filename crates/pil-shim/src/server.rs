//! OpenAI 互換 HTTP サーバ（DESIGN §4.1）．**`shim` feature でのみコンパイルされる**．
//!
//! Rust プロセスが `POST /v1/chat/completions`（＋ `GET /v1/models`）を localhost で立て，
//! 外部（Python）クライアントの `base_url` をここに向けさせる（制御反転）．全ての生成要求は
//! [`crate::mapping`] の純変換を通って `Arc<dyn LlmProvider>`（＝ pil-llm 単一経路）に集約され，
//! 温度・seed・max_tokens・system・metadata が全経路で揃う（§4.1）．非ストリーミングのみ（M1）．

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};

use pil_llm::LlmProvider;

use crate::mapping::{error_to_openai, to_chat_completion_response, to_generate_request};
use crate::openai::{ChatCompletionRequest, ModelList, ModelObject};

/// シムの共有状態（DESIGN §4.1）．
///
/// 背後の単一プロバイダと，`GET /v1/models` で広告するモデル一覧のみを持つ．
/// プロバイダは `Arc<dyn LlmProvider>` なので，Ollama native でもモックでも同じ経路で挿せる．
pub struct ShimState {
    provider: Arc<dyn LlmProvider>,
    models: Vec<String>,
}

impl ShimState {
    /// 背後プロバイダと広告モデル一覧からシム状態を作る．
    pub fn new(provider: Arc<dyn LlmProvider>, models: Vec<String>) -> Self {
        Self { provider, models }
    }
}

/// シムの axum ルータを組み立てる（§4.1）．
pub fn router(state: Arc<ShimState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        .with_state(state)
}

/// `POST /v1/chat/completions` ハンドラ．要求を [`to_generate_request`] で写し，単一プロバイダに
/// 委譲し，[`to_chat_completion_response`] で OpenAI 応答に戻す．失敗は OpenAI 形式のエラーに写す．
async fn chat_completions(
    State(state): State<Arc<ShimState>>,
    Json(request): Json<ChatCompletionRequest>,
) -> Response {
    let generate = to_generate_request(&request);
    match state.provider.generate(&generate).await {
        Ok(output) => {
            let body = to_chat_completion_response(&output, &request.model);
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(error) => {
            let (status, body) = error_to_openai(&error);
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, Json(body)).into_response()
        }
    }
}

/// `GET /v1/models` ハンドラ．構成時に渡された広告モデル一覧を返す．
async fn list_models(State(state): State<Arc<ShimState>>) -> Response {
    let data = state
        .models
        .iter()
        .map(|id| ModelObject {
            id: id.clone(),
            object: "model".to_string(),
            created: 0,
            owned_by: "pil-shim".to_string(),
        })
        .collect();

    let body = ModelList {
        object: "list".to_string(),
        data,
    };

    (StatusCode::OK, Json(body)).into_response()
}

/// 既に bind 済みの listener でシムを提供する（§4.1）．
///
/// テストは `127.0.0.1:0` でエフェメラルポートを bind し，`local_addr()` を取ってから本関数へ渡す．
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: Arc<ShimState>,
) -> std::io::Result<()> {
    axum::serve(listener, router(state).into_make_service()).await
}
