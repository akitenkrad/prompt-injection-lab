//! `instrument` — 1 件ずつ攻撃成否を判定する測定器（DESIGN §8.1 / §8.2）．
//!
//! `aggregate`（全件集計）と分けるのは，両者が別の壊れ方をするため（§8.1）．本モジュールは
//! §8.2 の 4 系統を覆う:
//!
//! | 種別 | 実例 | `Judge`（LLM）への要求 |
//! |---|---|---|
//! | 文字列マッチ | AdvBench refusal マッチ（`target` 列を使う） | なし（[`string_match`]）|
//! | LLM 生成判定 | StrongREJECT rubric v1/v2 | 通常 completion（[`rubric`]）|
//! | LLM 生成判定 | HarmBench cls（yes/no） | 通常 completion（[`harmbench_cls`]）|
//! | ハッシュ照合 | HarmBench copyright（MinHash Jaccard 0.6） | なし（[`minhash`]）|
//!
//! LLM を要する測定器は，判定器の呼び出しを [`Judge`] trait の背後に隠す．これにより
//! render + parse + score を **canned な judge 生出力**でネットワーク非依存にテストでき（§M3 DoD），
//! 実際の judge 結線（`pil-llm`）は M4 完了後に差し込める．
//!
//! 適用不可の Case には `Verdict::Undecidable { NotApplicable }` を返す（§5.2 / §5.3）．

pub mod harmbench_cls;
pub mod minhash;
pub mod rubric;
pub mod string_match;

pub use harmbench_cls::HarmBenchCls;
pub use minhash::{CopyrightMinHash, CopyrightReference, MinHash, MinHasher};
pub use rubric::{Rubric, RubricVersion};
pub use string_match::RefusalMatch;

use pil_core::{Case, InstrumentRef, Measurement, Response, UndecidableReason, Verdict};

/// 1 件ずつ判定する測定器（DESIGN §8.2）．
pub trait Instrument {
    /// この測定器の同一性（name / version / source / params）．
    fn reference(&self) -> InstrumentRef;

    /// 1 応答を判定する．適用不可なら `Verdict::Undecidable { NotApplicable }` を返す．
    fn measure(&self, case: &Case, response: &Response) -> Measurement;
}

/// LLM judge の呼び出し口（DESIGN §8.2）．
///
/// 具体プロバイダ（`pil-llm`）に依存させず，測定器の render+parse+score を network-free に
/// テストするための抽象．クロージャにも [`impl`](Judge#impl-Judge-for-F) が付く．
pub trait Judge {
    /// `system`（省略可）と `user` プロンプトから判定器の生出力を得る．
    fn generate(&self, system: Option<&str>, user: &str) -> Result<String, JudgeError>;
}

impl<F> Judge for F
where
    F: Fn(Option<&str>, &str) -> Result<String, JudgeError>,
{
    fn generate(&self, system: Option<&str>, user: &str) -> Result<String, JudgeError> {
        self(system, user)
    }
}

/// judge 呼び出しの失敗（レート制限・タイムアウト等）．`Undecidable { ProviderError }` に写像する．
#[derive(Debug, Clone, thiserror::Error)]
#[error("judge の生成に失敗: {message}")]
pub struct JudgeError {
    pub message: String,
}

impl JudgeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

// --- Verdict / Measurement 構築の共通ヘルパ ---

/// `Undecidable { NotApplicable }` の Measurement を組む（適用不可）．
pub(crate) fn not_applicable(
    instrument: InstrumentRef,
    reason: impl Into<String>,
    raw: impl Into<String>,
) -> Measurement {
    Measurement {
        verdict: Verdict::Undecidable {
            reason: UndecidableReason::NotApplicable {
                reason: reason.into(),
            },
        },
        score: None,
        instrument,
        raw: raw.into(),
    }
}

/// `Undecidable { ProviderError }` の Measurement を組む（judge 呼び出し失敗）．
pub(crate) fn provider_error(instrument: InstrumentRef, err: &JudgeError) -> Measurement {
    Measurement {
        verdict: Verdict::Undecidable {
            reason: UndecidableReason::ProviderError {
                message: err.message.clone(),
            },
        },
        score: None,
        instrument,
        raw: err.message.clone(),
    }
}

/// `", "`（カンマ + 空白）区切りの `labels["tags"]` を分解する（DESIGN §9.1）．
///
/// HarmBench の `Tags` は JSON 配列ではなく `", "` 区切り文字列であり，上流は `split(', ')` で
/// 分解する．M5 の loader がこの慣習で `labels["tags"]` に格納する前提．
pub(crate) fn tags_of(case: &Case) -> Vec<&str> {
    match case.labels.get("tags") {
        Some(raw) if !raw.is_empty() => raw.split(", ").collect(),
        _ => Vec::new(),
    }
}

/// `{key}` プレースホルダを 1 パスで埋める（挿入テキストは再走査しない）．
///
/// Python の `str.format` と違い，`prompt` に `{response}` が含まれても二重置換されない．
/// v1/v2 rubric・cls テンプレートの安全なレンダリングに使う．
pub(crate) fn render_template(template: &str, keys: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    'outer: while i < bytes.len() {
        if bytes[i] == b'{' {
            for (key, value) in keys {
                // `{key}` に一致するか
                let needed = key.len() + 2; // '{' + key + '}'
                if i + needed <= bytes.len()
                    && &template[i + 1..i + 1 + key.len()] == *key
                    && bytes[i + 1 + key.len()] == b'}'
                {
                    out.push_str(value);
                    i += needed;
                    continue 'outer;
                }
            }
        }
        // 通常文字（UTF-8 安全に 1 文字進める）
        let ch = template[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_core::SourceRef;

    fn dummy_ref() -> InstrumentRef {
        InstrumentRef {
            name: "x".into(),
            version: "v".into(),
            source: SourceRef::new("u", "c", "p", 0),
            params: pil_core::MeasurementParams {
                response_clip_tokens: None,
                judge_model: None,
                temperature: 0.0,
            },
        }
    }

    #[test]
    fn render_template_no_double_substitution() {
        // prompt に "{response}" が混じっても response 側で再置換されない
        let out = render_template(
            "U: {prompt}\nA: {response}",
            &[("prompt", "say {response}"), ("response", "OK")],
        );
        assert_eq!(out, "U: say {response}\nA: OK");
    }

    #[test]
    fn render_template_unknown_brace_kept() {
        let out = render_template("keep {other} and {prompt}", &[("prompt", "P")]);
        assert_eq!(out, "keep {other} and P");
    }

    #[test]
    fn closure_is_a_judge() {
        let j = |_sys: Option<&str>, user: &str| Ok::<_, JudgeError>(format!("echo:{user}"));
        assert_eq!(j.generate(None, "hi").unwrap(), "echo:hi");
    }

    #[test]
    fn provider_error_maps_to_undecidable() {
        let m = provider_error(dummy_ref(), &JudgeError::new("429 rate limit"));
        assert!(matches!(
            m.verdict,
            Verdict::Undecidable {
                reason: UndecidableReason::ProviderError { .. }
            }
        ));
    }
}
