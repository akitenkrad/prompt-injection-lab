//! `cross_env` — EnvKind 横断の提示（DESIGN §2.3 / §8.1 / §3.7）．
//!
//! §8.1 の要は「異なる `EnvKind` を跨いだ単一スコアを既定では出さない」ことである．本モジュールは
//! `by_env: BTreeMap<EnvKind, Vec<Trial>>` を入力に取り，二つの提示経路を型で分離する：
//!
//! - **既定（安全）**：[`per_env_view`] は `single_shot_asr_by_env` を並べるだけで，EnvKind を横断
//!   する単一スカラを構造として持たない（§8.1）．各 EnvKind の測定器別 ASR + CI + undecidable を
//!   横並びにする．
//! - **明示 opt-in（危険）**：[`cross_env_disclosure`] は [`CrossEnv`] マーカーの明示構築を要求し，
//!   指定測定器のプール ASR（`pool_asr_across_env`）に [`CROSS_ENV_WARNING`] を刻んだ上で返す．
//!   併せて **測定器跨ぎの Kendall W**（§3.7「結論はどの judge に依存するか」）を同梱し，比較不能性の
//!   度合いを開示する．出力は必ず警告文を可視で携える．

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use pil_core::{EnvKind, InstrumentRef, Trial};
use pil_metrics::aggregate::{
    instrument_concordance, pool_asr_across_env, single_shot_asr_by_env, CrossEnv, CrossEnvAsr,
    InstrumentConcordance, CROSS_ENV_WARNING,
};

use crate::present::AsrPresentation;

/// 単一 EnvKind の測定器別 ASR 提示（既定・安全経路の 1 ブロック）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvKindPresentation {
    pub env: EnvKind,
    /// 測定器ごとの ASR 提示（CI + undecidable を常時併記，§8.1）．
    pub per_instrument: Vec<(InstrumentRef, AsrPresentation)>,
}

/// EnvKind を**横並び**にした既定ビュー（DESIGN §8.1）．
///
/// **横断スカラを型として持たない**．跨いだ単一値が要るときは [`cross_env_disclosure`] の
/// 明示 opt-in を通す以外に道が無い．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerEnvView {
    /// EnvKind ごとのブロック（`EnvKind` の Ord 順）．
    pub envs: Vec<EnvKindPresentation>,
}

impl PerEnvView {
    /// 観測された EnvKind の一覧（監査用）．
    pub fn env_kinds(&self) -> Vec<EnvKind> {
        self.envs.iter().map(|e| e.env).collect()
    }

    /// 人間可読の横並び提示（DESIGN §8.1）．横断スカラは出さない．
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("EnvKind 横並び（跨いだ単一スコアは出さない，§8.1）\n");
        for block in &self.envs {
            out.push_str(&format!("  EnvKind = {:?}\n", block.env));
            if block.per_instrument.is_empty() {
                out.push_str("    （測定器なし）\n");
                continue;
            }
            for (inst, pres) in &block.per_instrument {
                out.push_str(&format!(
                    "    [{} {}] {}\n",
                    inst.name,
                    inst.version,
                    pres.render()
                ));
            }
        }
        out
    }
}

/// 既定（安全）ビューを構築する（DESIGN §8.1）．
///
/// `single_shot_asr_by_env` の結果を EnvKind ごとに横並びにするだけで，横断スカラは作らない．
pub fn per_env_view(by_env: &BTreeMap<EnvKind, Vec<Trial>>, z: f64) -> PerEnvView {
    let by_env_asr = single_shot_asr_by_env(by_env, z);
    let envs = by_env_asr
        .into_iter()
        .map(|(env, env_asr)| {
            let per_instrument = env_asr
                .per_instrument
                .iter()
                .map(|(inst, res)| (inst.clone(), AsrPresentation::from_asr_result(res, 95.0)))
                .collect();
            EnvKindPresentation {
                env,
                per_instrument,
            }
        })
        .collect();
    PerEnvView { envs }
}

/// EnvKind 横断の明示開示（危険，DESIGN §8.1 / §3.7）．
///
/// 必ず [`CROSS_ENV_WARNING`] を [`Self::warning`] に携える．`pooled` は指定測定器の環境跨ぎプール
/// ASR，`concordance` は測定器跨ぎの Kendall W（全 EnvKind の Trial をまとめて算出；測定器 2 未満・
/// 共通 Case 2 未満のときは `None`）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossEnvDisclosure {
    /// 比較不能の可能性を示す警告文（[`CROSS_ENV_WARNING`]）．常に可視で携える．
    pub warning: String,
    /// 環境跨ぎでプールした ASR（`pool_asr_across_env` の結果）．
    pub pooled: CrossEnvAsr,
    /// 測定器跨ぎの Kendall W（§3.7「結論はどの judge に依存するか」）．
    pub concordance: Option<InstrumentConcordance>,
}

impl CrossEnvDisclosure {
    /// 人間可読の開示（警告文と Kendall W を必ず可視で出す，DESIGN §8.1 / §3.7）．
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("[警告] {}\n", self.warning));
        out.push_str(&format!(
            "  環境跨ぎプール ASR [{} {}]（混合 EnvKind={:?}）: {}\n",
            self.pooled.instrument.name,
            self.pooled.instrument.version,
            self.pooled.envs,
            AsrPresentation::from_asr_result(&self.pooled.result, 95.0).render()
        ));
        match &self.concordance {
            Some(ic) => {
                out.push_str(&format!(
                    "  測定器跨ぎ Kendall W = {:.4}（測定器 {} 種・共通 Case {} 件・\
                     Undecidable 除外 {} 件，χ²≈{:.3} df={}）\n",
                    ic.kendall.w,
                    ic.instruments.len(),
                    ic.n_cases_used,
                    ic.n_cases_dropped_undecidable,
                    ic.kendall.chi_square,
                    ic.kendall.df,
                ));
                out.push_str(
                    "  → W が小さいほど「結論はどの測定器（judge）に依存するか」の度合いが大きい（§3.7）．\n",
                );
            }
            None => {
                out.push_str(
                    "  測定器跨ぎ Kendall W: 算出不能（測定器 2 未満または共通 Case 2 未満，§3.7）．\n",
                );
            }
        }
        out
    }
}

/// EnvKind 横断の明示開示を構築する（DESIGN §8.1 / §3.7）．
///
/// [`CrossEnv`] マーカーの明示構築を要求する（通常経路からは到達できない）．指定測定器のプール ASR に
/// [`CROSS_ENV_WARNING`] を刻み，測定器跨ぎの Kendall W（全 Trial をまとめて算出）を同梱する．
pub fn cross_env_disclosure(
    by_env: &BTreeMap<EnvKind, Vec<Trial>>,
    instrument: &InstrumentRef,
    z: f64,
    optin: CrossEnv,
) -> CrossEnvDisclosure {
    let pooled = pool_asr_across_env(by_env, instrument, z, optin);
    // 測定器跨ぎ一致度は全 EnvKind の Trial をまとめて算出する（judge 依存性の開示，§3.7）．
    let all_trials: Vec<Trial> = by_env.values().flat_map(|ts| ts.iter().cloned()).collect();
    let concordance = instrument_concordance(&all_trials);
    CrossEnvDisclosure {
        warning: CROSS_ENV_WARNING.to_string(),
        pooled,
        concordance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_core::{
        AttackRef, CaseId, FinishReason, Measurement, MeasurementParams, ModelRef, Response,
        SourceRef, UndecidableReason, Verdict,
    };
    use pil_metrics::aggregate::Z_95;

    fn inst(name: &str, version: &str) -> InstrumentRef {
        InstrumentRef {
            name: name.into(),
            version: version.into(),
            source: SourceRef::new("u", "c", "p", 0),
            params: MeasurementParams {
                response_clip_tokens: None,
                judge_model: None,
                temperature: 0.0,
            },
        }
    }

    fn cid(s: &str) -> CaseId {
        CaseId::derive(s, None, &SourceRef::new("u", "c", "p", 0))
    }

    fn resp() -> Response {
        Response {
            text: String::new(),
            finish_reason: FinishReason::Stop,
            prompt_tokens: None,
            completion_tokens: None,
            reached_clip_limit: false,
        }
    }

    /// 1 Case を 2 測定器で測る Trial．
    fn trial_two(
        case: &str,
        i1: &InstrumentRef,
        v1: Verdict,
        i2: &InstrumentRef,
        v2: Verdict,
    ) -> Trial {
        Trial {
            case: cid(case),
            attempt: 1,
            model: ModelRef::new("ollama", "m", None),
            attack: AttackRef::identity(),
            response: resp(),
            measurements: vec![
                Measurement {
                    verdict: v1,
                    score: None,
                    instrument: i1.clone(),
                    raw: String::new(),
                },
                Measurement {
                    verdict: v2,
                    score: None,
                    instrument: i2.clone(),
                    raw: String::new(),
                },
            ],
        }
    }

    fn undec() -> Verdict {
        Verdict::Undecidable {
            reason: UndecidableReason::ResponseTruncated,
        }
    }

    /// StaticPrompt と Emulated に跨る fixture（測定器 2 種・Undecidable を 1 件含む）．
    fn fixture() -> BTreeMap<EnvKind, Vec<Trial>> {
        let a = inst("rubric", "v1");
        let b = inst("rubric", "v2");
        let mut by_env = BTreeMap::new();
        by_env.insert(
            EnvKind::StaticPrompt,
            vec![
                trial_two("sp1", &a, Verdict::Success, &b, Verdict::Success),
                trial_two("sp2", &a, Verdict::Failure, &b, undec()), // b が判定不能
                trial_two("sp3", &a, Verdict::Success, &b, Verdict::Failure),
            ],
        );
        by_env.insert(
            EnvKind::Emulated,
            vec![
                trial_two("em1", &a, Verdict::Failure, &b, Verdict::Failure),
                trial_two("em2", &a, Verdict::Success, &b, Verdict::Success),
            ],
        );
        by_env
    }

    /// 既定ビューは両 EnvKind を横並びにし，横断スカラを持たない（§8.1）．
    #[test]
    fn default_view_places_envs_side_by_side_no_pooled_scalar() {
        let by_env = fixture();
        let view = per_env_view(&by_env, Z_95);
        // 両 EnvKind が並ぶ．
        assert_eq!(
            view.env_kinds(),
            vec![EnvKind::StaticPrompt, EnvKind::Emulated]
        );
        // 各 EnvKind に測定器別提示がある．
        for block in &view.envs {
            assert!(!block.per_instrument.is_empty());
        }
        // 横断スカラは構造として存在しない（PerEnvView に該当フィールドが無い）ことを render でも確認．
        let s = view.render();
        assert!(s.contains("StaticPrompt"), "{s}");
        assert!(s.contains("Emulated"), "{s}");
        assert!(
            !s.contains("環境跨ぎプール"),
            "既定に横断スカラが出てはならない: {s}"
        );
        assert!(!s.contains("警告"), "既定に警告は出ない: {s}");
    }

    /// opt-in 開示は警告文と Kendall W を可視で携える（§8.1 / §3.7）．
    #[test]
    fn optin_disclosure_carries_warning_and_kendall_w() {
        let by_env = fixture();
        let a = inst("rubric", "v1");
        let disc = cross_env_disclosure(&by_env, &a, Z_95, CrossEnv::i_understand_incomparable());
        assert_eq!(disc.warning, CROSS_ENV_WARNING);
        // 2 EnvKind をプールしている．
        assert_eq!(disc.pooled.envs.len(), 2);
        // 測定器 2 種・共通 Case が 2 件以上 → Kendall W が出る．
        let ic = disc.concordance.as_ref().expect("2 測定器・共通 Case あり");
        assert_eq!(ic.instruments.len(), 2);
        // sp2 は b が Undecidable → dropped=1．
        assert_eq!(ic.n_cases_dropped_undecidable, 1);
        assert!(ic.kendall.w.is_finite());
        // render は警告と W を可視で含む．
        let s = disc.render();
        assert!(s.contains(CROSS_ENV_WARNING), "{s}");
        assert!(s.contains("Kendall W"), "{s}");
        assert!(s.contains("環境跨ぎプール"), "{s}");
    }

    #[test]
    fn per_env_view_serde_roundtrip() {
        let by_env = fixture();
        let view = per_env_view(&by_env, Z_95);
        let json = serde_json::to_string(&view).unwrap();
        let back: PerEnvView = serde_json::from_str(&json).unwrap();
        // f64 は serde_json の往復で末尾 ULP が揺れるため（present.rs と同様），構造を比較する．
        assert_eq!(view.env_kinds(), back.env_kinds());
        assert_eq!(back.envs.len(), 2);
        for (a, b) in view.envs.iter().zip(&back.envs) {
            assert_eq!(a.per_instrument.len(), b.per_instrument.len());
            for ((ia, pa), (ib, pb)) in a.per_instrument.iter().zip(&b.per_instrument) {
                assert_eq!(ia, ib);
                assert_eq!(pa.successes, pb.successes);
                assert_eq!(pa.decided, pb.decided);
                assert_eq!(pa.undecidable, pb.undecidable);
                assert!((pa.asr - pb.asr).abs() < 1e-12);
            }
        }
    }
}
