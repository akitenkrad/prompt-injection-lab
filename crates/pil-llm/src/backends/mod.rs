//! ネットワークを要するバックエンド（DESIGN §6）．
//!
//! すべて cargo feature で gate する．既定（network-free）ビルドではこのモジュールは空になり，
//! reqwest を一切参照しない（§6.1）．

#[cfg(feature = "ollama")]
pub mod ollama;

#[cfg(feature = "openai")]
pub mod openai;

#[cfg(feature = "anthropic")]
pub mod anthropic;

#[cfg(feature = "gemini")]
pub mod gemini;
