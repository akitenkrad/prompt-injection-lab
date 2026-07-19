//! レンダリングエラー（DESIGN §5.6）．
//!
//! 変換のパラメータが Phase 1 の固定集合外のとき失敗させる（黙って劣化させない）．

use thiserror::Error;

/// `render` が失敗する条件．いずれもパラメータが固定集合外であることを表す．
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderError {
    /// `Translate{lang}` の言語タグが Phase 1 の固定オフライン集合に無い．
    #[error("unsupported translate language tag: {lang} (offline fixed set only)")]
    UnsupportedLang { lang: String },
    /// `Roleplay{template_id}` のテンプレート ID が固定表に無い．
    #[error("unknown roleplay template_id: {template_id}")]
    UnknownTemplate { template_id: String },
}
