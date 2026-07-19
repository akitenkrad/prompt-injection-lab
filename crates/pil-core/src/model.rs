//! `ModelRef` と `Response`（DESIGN §5.5 / §11.4）．
//!
//! `Response` は `ResponseTruncated`（§5.3）判定の根拠となる `finish_reason` / クリップ到達
//! フラグ / トークン数を保持する．

use serde::{Deserialize, Serialize};

/// 何のモデルと話したか．全呼び出しで記録する（§6.1 `CallMetadata` の中核）．
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRef {
    /// 例: `"ollama"` / `"openai"`
    pub provider: String,
    /// 例: `"llama3:8b"` / `"gpt-4-1106-preview"`
    pub model: String,
    /// 呼び出し先エンドポイント（既定 network-free ビルドのモック時は None）
    pub endpoint: Option<String>,
}

impl ModelRef {
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        endpoint: Option<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            endpoint,
        }
    }
}

/// 生成が停止した理由．`Length` はクリップ長到達で `ResponseTruncated` の根拠になる．
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishReason {
    /// 自然停止（stop トークン等）
    Stop,
    /// 最大トークン長に達して打ち切り
    Length,
    /// コンテンツフィルタで停止
    ContentFilter,
    /// tool 呼び出しで停止
    ToolCalls,
    /// その他（プロバイダ固有の生値を保持）
    Other(String),
}

/// 1 回の生成の応答（DESIGN §5.5 / §11.4）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub text: String,
    pub finish_reason: FinishReason,
    /// プロンプト側トークン数（プロバイダが返す場合）
    pub prompt_tokens: Option<u32>,
    /// 生成側トークン数（プロバイダが返す場合）
    pub completion_tokens: Option<u32>,
    /// 事前クリップ長（§3.9 の `--num_tokens 512` 等）に到達したか．
    /// `ResponseTruncated` 判定の直接の根拠．
    pub reached_clip_limit: bool,
}

impl Response {
    /// クリップ長到達 or `finish_reason == Length` のとき，応答は打ち切られたとみなす．
    pub fn is_truncated(&self) -> bool {
        self.reached_clip_limit || self.finish_reason == FinishReason::Length
    }
}
