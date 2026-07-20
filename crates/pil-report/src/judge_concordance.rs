//! `judge_concordance` — StrongREJECT 判定器の連続スコア一致度の提示（DESIGN §3.7）．
//!
//! §3.7 は「StrongREJECT の judge は 2 つある（rubric v1/v2）」という比較不能性を指摘する．本
//! モジュールはそれを **3 つ**（rubric v1 / rubric v2 / fine-tuned）へ拡張し，`Measurement.score`
//! の**連続値**を [`pil_metrics::aggregate::strongreject_score_concordance`] に載せて，グループの
//! Kendall W とペア別 W（v1↔v2 / v1↔fine-tuned / v2↔fine-tuned）を人間可読に提示する．
//!
//! 本モジュールは新しい統計を導入しない — 既存の M6 Kendall W をそのまま再利用する．StrongREJECT
//! 判定器が 2 種未満（例: Phase 1 は rubric v1 のみ）や共通スコア Case が 2 件未満のときは，比較
//! 対象が無い旨を注記し，**エラーにしない**．

use serde::{Deserialize, Serialize};

use pil_core::Trial;
use pil_metrics::aggregate::{strongreject_score_concordance, ScoreConcordance};

/// StrongREJECT 判定器一致度の提示（DESIGN §3.7）．
///
/// `concordance` が `None` のときは比較対象が無い（判定器 2 種未満・共通 Case 2 件未満）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeConcordance {
    /// 連続スコアに基づく測定器一致度（`None` なら比較対象なし）．
    pub concordance: Option<ScoreConcordance>,
}

impl JudgeConcordance {
    /// 人間可読の一致度提示（グループ W + ペア別 W + Case 件数，DESIGN §3.7）．
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("StrongREJECT 判定器一致度（連続スコア Kendall W，§3.7）\n");
        match &self.concordance {
            Some(sc) => {
                out.push_str(&format!(
                    "  グループ W = {:.4}（判定器 {} 種・共通スコア Case {} 件・\
                     Undecidable/スコア欠落 除外 {} 件，χ²≈{:.3} df={}）\n",
                    sc.group.w,
                    sc.instruments.len(),
                    sc.n_cases_used,
                    sc.n_cases_dropped,
                    sc.group.chi_square,
                    sc.group.df,
                ));
                out.push_str("  ペア別 W:\n");
                for (a, b, w) in &sc.pairwise {
                    out.push_str(&format!(
                        "    [{} {}] ↔ [{} {}]: W = {:.4}\n",
                        a.name, a.version, b.name, b.version, w.w,
                    ));
                }
                out.push_str(
                    "  → W が小さいほど「StrongREJECT スコアはどの実装（judge）に依存するか」の\
                     度合いが大きい（§3.7）．\n",
                );
            }
            None => {
                out.push_str(
                    "  比較対象が無い（StrongREJECT 判定器が 2 種未満，または共通スコア Case が \
                     2 件未満）．\n",
                );
                out.push_str(
                    "  例: Phase 1 は rubric v1 のみ．判定器が 2 種以上そろって初めて一致度を出せる．\n",
                );
            }
        }
        out
    }
}

/// StrongREJECT 判定器の連続スコア一致度提示を構築する（DESIGN §3.7）．
///
/// 全 Trial をまとめて [`strongreject_score_concordance`] に載せる．判定器 2 種未満・共通 Case
/// 2 件未満のときは `concordance = None` を携え，[`JudgeConcordance::render`] が注記を出す．
pub fn strongreject_judge_concordance(trials: &[Trial]) -> JudgeConcordance {
    JudgeConcordance {
        concordance: strongreject_score_concordance(trials),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_core::{
        AttackRef, CaseId, FinishReason, InstrumentRef, Measurement, MeasurementParams, ModelRef,
        Response, SourceRef, UndecidableReason, Verdict,
    };

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

    fn trial_scored(case: &str, cells: &[(&InstrumentRef, Verdict, Option<f64>)]) -> Trial {
        Trial {
            case: cid(case),
            attempt: 1,
            model: ModelRef::new("ollama", "m", None),
            attack: AttackRef::identity(),
            response: resp(),
            measurements: cells
                .iter()
                .map(|(i, v, s)| Measurement {
                    verdict: v.clone(),
                    score: *s,
                    instrument: (*i).clone(),
                    raw: String::new(),
                })
                .collect(),
        }
    }

    /// rubric v1 + v2 + fine-tuned が共通 Case を測る fixture → グループ + ペア別 W が出る．
    #[test]
    fn renders_group_and_pairwise_for_three_sr_judges() {
        let v1 = inst("strongreject-rubric", "v1");
        let v2 = inst("strongreject-rubric", "v2");
        let ft = inst("strongreject-finetuned", "15k-v1");
        let asc = [0.1, 0.2, 0.3, 0.4];
        let desc = [0.4, 0.3, 0.2, 0.1];
        let trials: Vec<Trial> = (0..4)
            .map(|idx| {
                trial_scored(
                    &format!("c{idx}"),
                    &[
                        (&v1, Verdict::Success, Some(asc[idx])),
                        (&v2, Verdict::Success, Some(asc[idx])),
                        (&ft, Verdict::Success, Some(desc[idx])),
                    ],
                )
            })
            .collect();
        let jc = strongreject_judge_concordance(&trials);
        let sc = jc.concordance.as_ref().expect("3 SR judges");
        assert_eq!(sc.instruments.len(), 3);
        assert_eq!(sc.pairwise.len(), 3);
        let s = jc.render();
        assert!(s.contains("グループ W"), "{s}");
        assert!(s.contains("ペア別 W"), "{s}");
        assert!(s.contains("strongreject-rubric"), "{s}");
        assert!(s.contains("strongreject-finetuned"), "{s}");
        // v1↔ft は反転で W=0.0，v1↔v2 は一致で W=1.0 が本文に現れる．
        assert!(s.contains("W = 1.0000"), "{s}");
        assert!(s.contains("W = 0.0000"), "{s}");
    }

    /// Undecidable を含む Case は除外され，除外件数が本文に出る．
    #[test]
    fn dropped_case_is_counted_in_render() {
        let v1 = inst("strongreject-rubric", "v1");
        let v2 = inst("strongreject-rubric", "v2");
        let good = |c: &str, s: f64| {
            trial_scored(
                c,
                &[
                    (&v1, Verdict::Success, Some(s)),
                    (&v2, Verdict::Success, Some(s)),
                ],
            )
        };
        let trials = vec![
            good("c1", 0.2),
            good("c2", 0.6),
            trial_scored(
                "c3",
                &[
                    (&v1, Verdict::Success, Some(0.5)),
                    (
                        &v2,
                        Verdict::Undecidable {
                            reason: UndecidableReason::ResponseTruncated,
                        },
                        None,
                    ),
                ],
            ),
        ];
        let jc = strongreject_judge_concordance(&trials);
        let sc = jc.concordance.as_ref().expect("2 SR judges");
        assert_eq!(sc.n_cases_dropped, 1);
        assert!(jc.render().contains("除外 1 件"), "{}", jc.render());
    }

    /// StrongREJECT 判定器が 1 種のみ → 「比較対象が無い」注記（エラーにしない，§3.7）．
    #[test]
    fn single_sr_judge_notes_nothing_to_compare() {
        let v1 = inst("strongreject-rubric", "v1");
        let trials = vec![
            trial_scored("c1", &[(&v1, Verdict::Success, Some(0.2))]),
            trial_scored("c2", &[(&v1, Verdict::Success, Some(0.8))]),
        ];
        let jc = strongreject_judge_concordance(&trials);
        assert!(jc.concordance.is_none());
        let s = jc.render();
        assert!(s.contains("比較対象が無い"), "{s}");
        assert!(s.contains("rubric v1 のみ"), "{s}");
    }

    #[test]
    fn serde_roundtrip() {
        let v1 = inst("strongreject-rubric", "v1");
        let v2 = inst("strongreject-rubric", "v2");
        let trials: Vec<Trial> = (0..3)
            .map(|idx| {
                let s = 0.2 + 0.3 * idx as f64;
                trial_scored(
                    &format!("c{idx}"),
                    &[
                        (&v1, Verdict::Success, Some(s)),
                        (&v2, Verdict::Success, Some(s)),
                    ],
                )
            })
            .collect();
        let jc = strongreject_judge_concordance(&trials);
        let json = serde_json::to_string(&jc).unwrap();
        let back: JudgeConcordance = serde_json::from_str(&json).unwrap();
        assert_eq!(jc.concordance.is_some(), back.concordance.is_some());
    }
}
