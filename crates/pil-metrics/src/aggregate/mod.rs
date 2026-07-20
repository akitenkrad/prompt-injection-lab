//! `aggregate` — ASR・信頼区間・union coverage・多試行 ASR と，比較可能性の型強制（DESIGN §8.1）．
//!
//! 「この応答は攻撃成功か」を 1 件ずつ判定する `instrument` に対し，本モジュールは
//! **全件から集計**する（§8.1）．両者は別の壊れ方をする（前者は再現率で，後者は少サンプルで）．
//!
//! 本モジュールの要は **`EnvKind` 比較可能性の型強制**（§2.3 / §8.1）である．集計入力は
//! `by_env: BTreeMap<EnvKind, Vec<Trial>>` の形を取り，戻り値は必ず `EnvKind` でタグ付けした
//! マップになる．**異なる `EnvKind` を横断する単一スコアは通常 API から出せない** — 跨ぐには
//! [`CrossEnv`] マーカーを明示的に構築せねばならず，出力には [`CROSS_ENV_WARNING`] を必ず刻む．
//! 同様に測定器（[`InstrumentRef`]）でグルーピングし，測定器跨ぎの単純平均も [`CrossInstrument`]
//! の明示 opt-in を要求する（§3.7 の「別測定器の数字を同じ土俵に載せる」を防ぐ）．
//!
//! 統計は §11.3 に従う：単発 ASR（二項割合）は Wilson 区間，union/多試行など単純割合でない
//! 統計は Case 単位 bootstrap（percentile/BCa，seeded）で出す．`Undecidable` は分母から除外し
//! 件数を保持する（§5.3）．

pub mod bootstrap;
pub mod concordance;
pub mod interval;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use pil_core::{AttackRef, CaseId, EnvKind, InstrumentRef, Trial, Verdict};

pub use bootstrap::{bootstrap_ci, BootstrapMethod};
pub use concordance::{instrument_concordance, kendall_w, InstrumentConcordance, KendallW};
pub use interval::{
    clopper_pearson_interval, normal_cdf, normal_quantile, wilson_interval, ConfidenceInterval,
    Z_95,
};

/// 環境種別跨ぎ集計に刻む警告文（DESIGN §8.1）．
pub const CROSS_ENV_WARNING: &str = "環境種別跨ぎ（比較不能の可能性）";

/// 測定器跨ぎの単純平均に刻む警告文（DESIGN §8.1 / §3.7）．
pub const CROSS_INSTRUMENT_WARNING: &str = "測定器跨ぎの単純平均（比較不能の可能性）";

// ---------------------------------------------------------------------------
// 単発 ASR（§11.3 / §5.3）
// ---------------------------------------------------------------------------

/// 単一の `(EnvKind, InstrumentRef)` 群での ASR（DESIGN §5.3 / §11.3）．
///
/// `asr = successes / (successes + failures)`．`Undecidable` は分母から除外するが
/// [`Self::undecidable`] に件数を保持する（§5.3）．`ci` は Wilson score 区間．
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AsrResult {
    pub successes: usize,
    pub failures: usize,
    /// 分母から除外した `Undecidable` の件数（§5.3 で必ず併記する）．
    pub undecidable: usize,
    /// `successes / (successes + failures)`．判定可能が 0 件なら `NaN`．
    pub asr: f64,
    /// Wilson score 区間（既定，§11.3）．
    pub ci: ConfidenceInterval,
}

fn asr_from_verdicts<'a>(verdicts: impl Iterator<Item = &'a Verdict>, z: f64) -> AsrResult {
    let (mut successes, mut failures, mut undecidable) = (0usize, 0usize, 0usize);
    for v in verdicts {
        match v {
            Verdict::Success => successes += 1,
            Verdict::Failure => failures += 1,
            Verdict::Undecidable { .. } => undecidable += 1,
        }
    }
    let denom = successes + failures;
    let asr = if denom == 0 {
        f64::NAN
    } else {
        successes as f64 / denom as f64
    };
    AsrResult {
        successes,
        failures,
        undecidable,
        asr,
        ci: wilson_interval(successes, denom, z),
    }
}

/// ある Trial 内で，指定測定器に一致する測定の verdict を列挙する（§5.4 の同一性で照合）．
fn verdicts_for<'a>(
    trial: &'a Trial,
    instrument: &'a InstrumentRef,
) -> impl Iterator<Item = &'a Verdict> {
    trial
        .measurements
        .iter()
        .filter(move |m| &m.instrument == instrument)
        .map(|m| &m.verdict)
}

/// Trial 群に現れる測定器を出現順に一意化する（`InstrumentRef` は `Ord`/`Hash` を持たないため線形）．
fn distinct_instruments(trials: &[Trial]) -> Vec<InstrumentRef> {
    let mut out: Vec<InstrumentRef> = Vec::new();
    for t in trials {
        for m in &t.measurements {
            if !out.iter().any(|i| i == &m.instrument) {
                out.push(m.instrument.clone());
            }
        }
    }
    out
}

/// 単一 `EnvKind` の ASR 集計（測定器ごと）．結果は `env` でタグ付けされる（§8.1）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvAsr {
    pub env: EnvKind,
    /// 測定器ごとの ASR．測定器跨ぎの単純平均は既定で出さない（§8.1 / §3.7）．
    pub per_instrument: Vec<(InstrumentRef, AsrResult)>,
}

/// 単発 ASR を **`EnvKind` ごと**に集計する（DESIGN §8.1 の通常 API）．
///
/// 戻り値は `EnvKind` をキーとするマップであり，**環境種別を跨いだ単一スカラは出せない**．
/// 跨ぐには [`pool_asr_across_env`] に [`CrossEnv`] マーカーを渡す必要がある．
pub fn single_shot_asr_by_env(
    by_env: &BTreeMap<EnvKind, Vec<Trial>>,
    z: f64,
) -> BTreeMap<EnvKind, EnvAsr> {
    by_env
        .iter()
        .map(|(env, trials)| {
            let per_instrument = distinct_instruments(trials)
                .into_iter()
                .map(|inst| {
                    let res =
                        asr_from_verdicts(trials.iter().flat_map(|t| verdicts_for(t, &inst)), z);
                    (inst, res)
                })
                .collect();
            (
                *env,
                EnvAsr {
                    env: *env,
                    per_instrument,
                },
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// EnvKind / InstrumentRef 跨ぎの明示 opt-in（§8.1）
// ---------------------------------------------------------------------------

/// 環境種別跨ぎ集計を許可する明示マーカー（DESIGN §8.1）．
///
/// 通常 API（`*_by_env`）は結果を必ず `EnvKind` でタグ付けし，横断スカラを返さない．
/// この型は [`CrossEnv::i_understand_incomparable`] からしか作れないため，「跨ぐ」という
/// 意思決定を型で強制する（Kendall W = 0.10 の比較不能性を黙って踏み抜けないようにする，§2.3）．
#[derive(Debug, Clone, Copy)]
pub struct CrossEnv {
    _private: (),
}

impl CrossEnv {
    /// 「環境種別跨ぎは比較不能かもしれない」と理解した上でのみ構築できる（§2.3 / §8.1）．
    pub fn i_understand_incomparable() -> Self {
        Self { _private: () }
    }
}

/// 環境種別を跨いでプールした ASR（危険）．必ず [`CROSS_ENV_WARNING`] を刻む（§8.1）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossEnvAsr {
    /// 比較不能の可能性を示す警告文（[`CROSS_ENV_WARNING`]）．
    pub warning: String,
    /// プールした環境種別の一覧（何を混ぜたかの監査用）．
    pub envs: Vec<EnvKind>,
    pub instrument: InstrumentRef,
    pub result: AsrResult,
}

/// 環境種別を跨いで単一測定器の ASR をプールする（DESIGN §8.1 の unsafe 経路）．
///
/// [`CrossEnv`] マーカーの明示構築を要求し，戻り値に [`CROSS_ENV_WARNING`] を刻む．
/// 通常の集計では決して呼ばれない — 比較可能性の保証を意図的に外すときだけ使う．
pub fn pool_asr_across_env(
    by_env: &BTreeMap<EnvKind, Vec<Trial>>,
    instrument: &InstrumentRef,
    z: f64,
    _optin: CrossEnv,
) -> CrossEnvAsr {
    let envs: Vec<EnvKind> = by_env.keys().copied().collect();
    let result = asr_from_verdicts(
        by_env
            .values()
            .flat_map(|ts| ts.iter())
            .flat_map(|t| verdicts_for(t, instrument)),
        z,
    );
    CrossEnvAsr {
        warning: CROSS_ENV_WARNING.to_string(),
        envs,
        instrument: instrument.clone(),
        result,
    }
}

/// 測定器跨ぎの単純平均を許可する明示マーカー（DESIGN §8.1 / §3.7）．
#[derive(Debug, Clone, Copy)]
pub struct CrossInstrument {
    _private: (),
}

impl CrossInstrument {
    /// 「測定器が違えば土俵が違う（§3.7）」と理解した上でのみ構築できる．
    pub fn i_understand_instruments_differ() -> Self {
        Self { _private: () }
    }
}

/// 測定器跨ぎの単純平均 ASR（危険）．必ず [`CROSS_INSTRUMENT_WARNING`] を刻む（§8.1 / §3.7）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossInstrumentAsr {
    pub warning: String,
    pub env: EnvKind,
    pub instruments: Vec<InstrumentRef>,
    /// 測定器ごとの ASR の単純平均（判定可能が 0 件の測定器は除外）．
    pub mean_asr: f64,
}

/// 単一 `EnvKind` 内で，測定器を跨いだ ASR の単純平均を取る（DESIGN §8.1 の unsafe 経路）．
///
/// [`CrossInstrument`] マーカーの明示構築を要求し，[`CROSS_INSTRUMENT_WARNING`] を刻む．
pub fn mean_asr_across_instruments(
    env_asr: &EnvAsr,
    _optin: CrossInstrument,
) -> CrossInstrumentAsr {
    let asrs: Vec<f64> = env_asr
        .per_instrument
        .iter()
        .filter_map(|(_, r)| r.asr.is_finite().then_some(r.asr))
        .collect();
    let mean_asr = if asrs.is_empty() {
        f64::NAN
    } else {
        asrs.iter().sum::<f64>() / asrs.len() as f64
    };
    CrossInstrumentAsr {
        warning: CROSS_INSTRUMENT_WARNING.to_string(),
        env: env_asr.env,
        instruments: env_asr
            .per_instrument
            .iter()
            .map(|(i, _)| i.clone())
            .collect(),
        mean_asr,
    }
}

// ---------------------------------------------------------------------------
// union coverage（§2.2 / §5.6）
// ---------------------------------------------------------------------------

/// 攻撃バリアント跨ぎの union coverage（DESIGN §5.6）．
///
/// ある Case について変換集合 `V` を当て，`coverage(case) = 1` iff `∃ v∈V. Success`．
/// behavior 群の union ASR は `mean_case(coverage)`．`single_best_asr` は単一最良変換の ASR で，
/// §2.2 の「単一最良 < union」対比を同一構造で示すために併記する．全変換で `Undecidable` のみ
/// だった Case は分母から除外し件数を保持する（§5.3）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnionCoverage {
    pub env: EnvKind,
    pub instrument: InstrumentRef,
    /// union の分母（判定可能な Case 数）．
    pub n_cases: usize,
    /// 全変換で `Undecidable` のみだった Case（除外・件数保持，§5.3）．
    pub undecidable_only_cases: usize,
    /// `mean_case(coverage)`（§5.6）．
    pub union_asr: f64,
    /// 単一最良変換の ASR（§2.2 の対比）．
    pub single_best_asr: f64,
    /// 変換ごとの単発 ASR（監査・対比用）．
    pub per_variant_asr: Vec<(AttackRef, f64)>,
    /// `union_asr` の Case 単位 bootstrap 区間（percentile，§11.3）．
    pub union_ci: ConfidenceInterval,
}

/// 単一 `(EnvKind, InstrumentRef)` 群の union coverage を計算する（DESIGN §5.6）．
pub fn union_coverage_for(
    trials: &[Trial],
    env: EnvKind,
    instrument: &InstrumentRef,
    n_boot: usize,
    alpha: f64,
    seed: u64,
) -> UnionCoverage {
    // CaseId -> [(AttackRef, Verdict)]（CaseId は Ord なので BTreeMap で決定論的順序）
    let mut by_case: BTreeMap<CaseId, Vec<(AttackRef, Verdict)>> = BTreeMap::new();
    for t in trials {
        for m in &t.measurements {
            if &m.instrument == instrument {
                by_case
                    .entry(t.case.clone())
                    .or_default()
                    .push((t.attack.clone(), m.verdict.clone()));
            }
        }
    }

    // 変換ごとの (success, decided) を出現順に集計
    let mut variant_tally: Vec<(AttackRef, usize, usize)> = Vec::new();
    // Case 単位の coverage（判定可能な Case のみ）
    let mut case_units: Vec<u8> = Vec::new();
    let mut undecidable_only_cases = 0usize;

    for verdicts in by_case.values() {
        let mut any_success = false;
        let mut any_failure = false;
        for (attack, v) in verdicts {
            match v {
                Verdict::Success => any_success = true,
                Verdict::Failure => any_failure = true,
                Verdict::Undecidable { .. } => {}
            }
            let slot = match variant_tally.iter_mut().find(|(a, _, _)| a == attack) {
                Some(slot) => slot,
                None => {
                    variant_tally.push((attack.clone(), 0, 0));
                    variant_tally.last_mut().expect("just pushed")
                }
            };
            match v {
                Verdict::Success => {
                    slot.1 += 1;
                    slot.2 += 1;
                }
                Verdict::Failure => slot.2 += 1,
                Verdict::Undecidable { .. } => {}
            }
        }
        if any_success {
            case_units.push(1);
        } else if any_failure {
            case_units.push(0);
        } else {
            undecidable_only_cases += 1;
        }
    }

    let n_cases = case_units.len();
    let per_variant_asr: Vec<(AttackRef, f64)> = variant_tally
        .iter()
        .map(|(a, s, d)| {
            let asr = if *d == 0 {
                f64::NAN
            } else {
                *s as f64 / *d as f64
            };
            (a.clone(), asr)
        })
        .collect();
    let single_best_asr = per_variant_asr
        .iter()
        .filter_map(|(_, v)| v.is_finite().then_some(*v))
        .fold(f64::NEG_INFINITY, f64::max);
    let single_best_asr = if single_best_asr.is_finite() {
        single_best_asr
    } else {
        f64::NAN
    };

    let union_ci = bootstrap_ci(
        &case_units,
        |sample| {
            if sample.is_empty() {
                return f64::NAN;
            }
            let mut total = 0.0;
            for x in sample {
                total += **x as f64;
            }
            total / sample.len() as f64
        },
        n_boot,
        alpha,
        seed,
        BootstrapMethod::Percentile,
    );

    UnionCoverage {
        env,
        instrument: instrument.clone(),
        n_cases,
        undecidable_only_cases,
        union_asr: union_ci.point,
        single_best_asr,
        per_variant_asr,
        union_ci,
    }
}

/// union coverage を **`EnvKind` ごと・測定器ごと**に集計する（DESIGN §8.1 の通常 API）．
pub fn union_coverage_by_env(
    by_env: &BTreeMap<EnvKind, Vec<Trial>>,
    n_boot: usize,
    alpha: f64,
    seed: u64,
) -> BTreeMap<EnvKind, Vec<UnionCoverage>> {
    by_env
        .iter()
        .map(|(env, trials)| {
            let per_instrument = distinct_instruments(trials)
                .into_iter()
                .map(|inst| union_coverage_for(trials, *env, &inst, n_boot, alpha, seed))
                .collect();
            (*env, per_instrument)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 多試行 ASR 曲線（Anthropic 式 1/10/100，§11.3）
// ---------------------------------------------------------------------------

/// `asr@k` の 1 点．
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AsrAtK {
    pub k: u32,
    /// 最初の k 試行以内に 1 回でも Success した Case の割合．
    pub asr: f64,
    /// Case 単位 bootstrap 区間（percentile，§11.3）．
    pub ci: ConfidenceInterval,
}

/// 多試行 ASR 曲線（DESIGN §11.3）．
///
/// Case ごとに attempt 別の verdict を見て，`asr@k` = 「最初の k 試行以内に ≥1 Success した
/// Case の割合」を，呼び出し側が与える k 集合上で出す．全 attempt が `Undecidable` の Case は
/// 分母から除外し件数を保持する（§5.3）．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsrCurve {
    pub env: EnvKind,
    pub instrument: InstrumentRef,
    /// 判定可能な Case 数（曲線の分母）．
    pub n_cases: usize,
    /// 全 attempt が `Undecidable` だった Case（除外・件数保持，§5.3）．
    pub undecidable_only_cases: usize,
    pub points: Vec<AsrAtK>,
}

/// 単一 `(EnvKind, InstrumentRef)` 群の多試行 ASR 曲線を計算する（DESIGN §11.3）．
pub fn asr_curve_for(
    trials: &[Trial],
    env: EnvKind,
    instrument: &InstrumentRef,
    ks: &[u32],
    n_boot: usize,
    alpha: f64,
    seed: u64,
) -> AsrCurve {
    // CaseId -> (最初に Success した attempt, 判定可能か)
    let mut by_case: BTreeMap<CaseId, (Option<u32>, bool)> = BTreeMap::new();
    for t in trials {
        for m in &t.measurements {
            if &m.instrument == instrument {
                let entry = by_case.entry(t.case.clone()).or_insert((None, false));
                match &m.verdict {
                    Verdict::Success => {
                        entry.1 = true;
                        entry.0 = Some(entry.0.map_or(t.attempt, |a| a.min(t.attempt)));
                    }
                    Verdict::Failure => entry.1 = true,
                    Verdict::Undecidable { .. } => {}
                }
            }
        }
    }

    // 判定可能な Case のみ（各要素 = 最初に Success した attempt，無ければ None）
    let decided: Vec<Option<u32>> = by_case
        .values()
        .filter(|(_, decided)| *decided)
        .map(|(first, _)| *first)
        .collect();
    let undecidable_only_cases = by_case.values().filter(|(_, decided)| !*decided).count();
    let n_cases = decided.len();

    let points = ks
        .iter()
        .map(|&k| {
            let stat = |sample: &[&Option<u32>]| -> f64 {
                if sample.is_empty() {
                    return f64::NAN;
                }
                let mut covered = 0usize;
                for m in sample {
                    if let Some(a) = **m {
                        if a <= k {
                            covered += 1;
                        }
                    }
                }
                covered as f64 / sample.len() as f64
            };
            // k ごとに seed をずらして相関を避ける（決定論は維持）
            let ci = bootstrap_ci(
                &decided,
                stat,
                n_boot,
                alpha,
                seed.wrapping_add(k as u64),
                BootstrapMethod::Percentile,
            );
            AsrAtK {
                k,
                asr: ci.point,
                ci,
            }
        })
        .collect();

    AsrCurve {
        env,
        instrument: instrument.clone(),
        n_cases,
        undecidable_only_cases,
        points,
    }
}

/// 多試行 ASR 曲線を **`EnvKind` ごと・測定器ごと**に集計する（DESIGN §8.1 の通常 API）．
pub fn asr_curve_by_env(
    by_env: &BTreeMap<EnvKind, Vec<Trial>>,
    ks: &[u32],
    n_boot: usize,
    alpha: f64,
    seed: u64,
) -> BTreeMap<EnvKind, Vec<AsrCurve>> {
    by_env
        .iter()
        .map(|(env, trials)| {
            let per_instrument = distinct_instruments(trials)
                .into_iter()
                .map(|inst| asr_curve_for(trials, *env, &inst, ks, n_boot, alpha, seed))
                .collect();
            (*env, per_instrument)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_core::{
        AttackRef, FinishReason, InstrumentRef, Measurement, MeasurementParams, ModelRef, Response,
        SourceRef, Transform, UndecidableReason,
    };

    // --- テスト用ビルダ ---

    fn inst(name: &str, version: &str) -> InstrumentRef {
        InstrumentRef {
            name: name.into(),
            version: version.into(),
            source: SourceRef::new("upstream", "commit", "path", 0),
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

    fn trial(case: &str, attempt: u32, attack: AttackRef, i: InstrumentRef, v: Verdict) -> Trial {
        Trial {
            case: cid(case),
            attempt,
            model: ModelRef::new("ollama", "m", None),
            attack,
            response: resp(),
            measurements: vec![Measurement {
                verdict: v,
                score: None,
                instrument: i,
                raw: String::new(),
            }],
        }
    }

    fn undec() -> Verdict {
        Verdict::Undecidable {
            reason: UndecidableReason::ResponseTruncated,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    // --- Wilson テストベクタ（M8 DoD） ---

    #[test]
    fn wilson_known_vectors() {
        // 50/100 ≈ [0.4038, 0.5962]
        let ci = wilson_interval(50, 100, Z_95);
        assert!(close(ci.lower, 0.4038), "lower={}", ci.lower);
        assert!(close(ci.upper, 0.5962), "upper={}", ci.upper);
        assert!(close(ci.point, 0.5));

        // 0/10 ≈ [0.0, 0.2775]
        let ci = wilson_interval(0, 10, Z_95);
        assert!(close(ci.lower, 0.0), "lower={}", ci.lower);
        assert!(close(ci.upper, 0.2775), "upper={}", ci.upper);

        // 10/10 ≈ [0.7225, 1.0]
        let ci = wilson_interval(10, 10, Z_95);
        assert!(close(ci.lower, 0.7225), "lower={}", ci.lower);
        assert!(close(ci.upper, 1.0), "upper={}", ci.upper);
    }

    #[test]
    fn wilson_and_cp_edges_n_zero() {
        let w = wilson_interval(0, 0, Z_95);
        assert!(w.point.is_nan() && w.lower == 0.0 && w.upper == 1.0);
        let cp = clopper_pearson_interval(0, 0, 0.05);
        assert!(cp.point.is_nan() && cp.lower == 0.0 && cp.upper == 1.0);
    }

    #[test]
    fn clopper_pearson_known_vectors() {
        // 0/10: 下限 0，上限 = 1 - 0.025^(1/10) ≈ 0.3085
        let cp = clopper_pearson_interval(0, 10, 0.05);
        assert_eq!(cp.lower, 0.0);
        assert!(close(cp.upper, 0.3085), "upper={}", cp.upper);
        // 10/10: 下限 = 0.025^(1/10) ≈ 0.6915，上限 1
        let cp = clopper_pearson_interval(10, 10, 0.05);
        assert!(close(cp.lower, 0.6915), "lower={}", cp.lower);
        assert_eq!(cp.upper, 1.0);
    }

    // --- bootstrap: 退化と決定論（M8 DoD） ---

    #[test]
    fn bootstrap_degenerate_all_success_collapses() {
        // 全成功の coverage → 分布が 1.0 に潰れ CI は [1.0, 1.0]
        let units = vec![1u8, 1, 1, 1, 1];
        let ci = bootstrap_ci(
            &units,
            |s| s.iter().map(|&&x| x as f64).sum::<f64>() / s.len() as f64,
            1000,
            0.05,
            42,
            BootstrapMethod::Percentile,
        );
        assert_eq!(ci.point, 1.0);
        assert_eq!(ci.lower, 1.0);
        assert_eq!(ci.upper, 1.0);
    }

    #[test]
    fn bootstrap_is_deterministic_for_fixed_seed() {
        let units = vec![1u8, 0, 1, 0, 1, 1, 0, 0, 1, 0];
        let stat = |s: &[&u8]| s.iter().map(|&&x| x as f64).sum::<f64>() / s.len() as f64;
        // 同一 seed ⇒ 2 回とも完全一致（決定論，M8 DoD）
        let a = bootstrap_ci(&units, stat, 500, 0.05, 7, BootstrapMethod::Percentile);
        let b = bootstrap_ci(&units, stat, 500, 0.05, 7, BootstrapMethod::Percentile);
        assert_eq!(a, b);
    }

    #[test]
    fn bootstrap_bca_falls_back_when_degenerate() {
        // 全成功では BCa の z0 が定義できず percentile にフォールバック → [1,1]
        let units = vec![1u8; 8];
        let ci = bootstrap_ci(
            &units,
            |s| s.iter().map(|&&x| x as f64).sum::<f64>() / s.len() as f64,
            300,
            0.05,
            1,
            BootstrapMethod::Bca,
        );
        assert_eq!(ci.lower, 1.0);
        assert_eq!(ci.upper, 1.0);
    }

    // --- 単発 ASR: Undecidable は分母から除外し件数保持（§5.3, M8 DoD） ---

    #[test]
    fn undecidable_excluded_from_denominator_count_preserved() {
        let i = inst("string-match", "v1");
        let mut trials = Vec::new();
        for n in 0..3 {
            trials.push(trial(
                &format!("s{n}"),
                1,
                AttackRef::identity(),
                i.clone(),
                Verdict::Success,
            ));
        }
        for n in 0..2 {
            trials.push(trial(
                &format!("f{n}"),
                1,
                AttackRef::identity(),
                i.clone(),
                Verdict::Failure,
            ));
        }
        for n in 0..4 {
            trials.push(trial(
                &format!("u{n}"),
                1,
                AttackRef::identity(),
                i.clone(),
                undec(),
            ));
        }
        let by_env = BTreeMap::from([(EnvKind::StaticPrompt, trials)]);
        let out = single_shot_asr_by_env(&by_env, Z_95);
        let res = &out[&EnvKind::StaticPrompt].per_instrument[0].1;
        assert_eq!(res.successes, 3);
        assert_eq!(res.failures, 2);
        assert_eq!(res.undecidable, 4); // 件数は保持
        assert!(close(res.asr, 0.6)); // 分母 = 5（undecidable 除外）
    }

    // --- union coverage: 単一最良 < union（§2.2, M8 DoD） ---

    #[test]
    fn union_coverage_exceeds_single_best_variant() {
        let i = inst("rubric", "v1");
        let a = AttackRef::new(Transform::Identity, None);
        let b = AttackRef::new(Transform::Base64, None);
        // case1: A 成功 / B 失敗，case2: A 失敗 / B 成功
        let trials = vec![
            trial("case1", 1, a.clone(), i.clone(), Verdict::Success),
            trial("case1", 1, b.clone(), i.clone(), Verdict::Failure),
            trial("case2", 1, a.clone(), i.clone(), Verdict::Failure),
            trial("case2", 1, b.clone(), i.clone(), Verdict::Success),
        ];
        let uc = union_coverage_for(&trials, EnvKind::StaticPrompt, &i, 500, 0.05, 99);
        assert_eq!(uc.n_cases, 2);
        assert_eq!(uc.undecidable_only_cases, 0);
        assert!(
            close(uc.single_best_asr, 0.5),
            "best={}",
            uc.single_best_asr
        );
        assert!(close(uc.union_asr, 1.0), "union={}", uc.union_asr);
        assert!(uc.union_asr > uc.single_best_asr);
        // 各変換の単発 ASR は 0.5
        for (_, v) in &uc.per_variant_asr {
            assert!(close(*v, 0.5));
        }
    }

    #[test]
    fn union_coverage_undecidable_only_case_excluded_counted() {
        let i = inst("rubric", "v1");
        let a = AttackRef::new(Transform::Identity, None);
        let b = AttackRef::new(Transform::Base64, None);
        let trials = vec![
            // decided case
            trial("c1", 1, a.clone(), i.clone(), Verdict::Success),
            // undecidable-only case（両変換とも判定不能）
            trial("c2", 1, a.clone(), i.clone(), undec()),
            trial("c2", 1, b.clone(), i.clone(), undec()),
        ];
        let uc = union_coverage_for(&trials, EnvKind::StaticPrompt, &i, 200, 0.05, 1);
        assert_eq!(uc.n_cases, 1);
        assert_eq!(uc.undecidable_only_cases, 1);
        assert!(close(uc.union_asr, 1.0));
    }

    // --- 多試行 ASR 曲線（§11.3） ---

    #[test]
    fn asr_curve_is_monotone_nondecreasing() {
        let i = inst("string-match", "v1");
        let a = AttackRef::identity();
        // case1: attempt1 失敗 → attempt2 成功（最初の成功 = 2）
        // case2: attempt1 成功（最初の成功 = 1）
        let trials = vec![
            trial("c1", 1, a.clone(), i.clone(), Verdict::Failure),
            trial("c1", 2, a.clone(), i.clone(), Verdict::Success),
            trial("c2", 1, a.clone(), i.clone(), Verdict::Success),
        ];
        let curve = asr_curve_for(&trials, EnvKind::StaticPrompt, &i, &[1, 2, 3], 300, 0.05, 5);
        assert_eq!(curve.n_cases, 2);
        // asr@1 = 0.5（c2 のみ），asr@2 = 1.0，asr@3 = 1.0
        assert!(
            close(curve.points[0].asr, 0.5),
            "@1={}",
            curve.points[0].asr
        );
        assert!(
            close(curve.points[1].asr, 1.0),
            "@2={}",
            curve.points[1].asr
        );
        assert!(
            close(curve.points[2].asr, 1.0),
            "@3={}",
            curve.points[2].asr
        );
        // 単調非減少
        assert!(curve.points[0].asr <= curve.points[1].asr);
        assert!(curve.points[1].asr <= curve.points[2].asr);
    }

    // --- EnvKind 強制（§8.1, M8 DoD） ---

    #[test]
    fn normal_api_yields_per_env_never_cross_env_scalar() {
        let i = inst("string-match", "v1");
        let by_env = BTreeMap::from([
            (
                EnvKind::StaticPrompt,
                vec![trial(
                    "a",
                    1,
                    AttackRef::identity(),
                    i.clone(),
                    Verdict::Success,
                )],
            ),
            (
                EnvKind::Emulated,
                vec![trial(
                    "b",
                    1,
                    AttackRef::identity(),
                    i.clone(),
                    Verdict::Failure,
                )],
            ),
        ]);
        // 通常 API は EnvKind ごとのマップを返す — 横断スカラは型として存在しない
        let out: BTreeMap<EnvKind, EnvAsr> = single_shot_asr_by_env(&by_env, Z_95);
        assert_eq!(out.len(), 2);
        assert!(out.contains_key(&EnvKind::StaticPrompt));
        assert!(out.contains_key(&EnvKind::Emulated));
        // 各結果は自らの EnvKind でタグ付けされている
        assert_eq!(out[&EnvKind::StaticPrompt].env, EnvKind::StaticPrompt);
        assert_eq!(out[&EnvKind::Emulated].env, EnvKind::Emulated);
    }

    #[test]
    fn cross_env_pool_requires_marker_and_stamps_warning() {
        let i = inst("string-match", "v1");
        let by_env = BTreeMap::from([
            (
                EnvKind::StaticPrompt,
                vec![trial(
                    "a",
                    1,
                    AttackRef::identity(),
                    i.clone(),
                    Verdict::Success,
                )],
            ),
            (
                EnvKind::RealExecutable,
                vec![trial(
                    "b",
                    1,
                    AttackRef::identity(),
                    i.clone(),
                    Verdict::Failure,
                )],
            ),
        ]);
        // 明示マーカーが無ければ呼べない（型で強制）
        let pooled = pool_asr_across_env(&by_env, &i, Z_95, CrossEnv::i_understand_incomparable());
        assert_eq!(pooled.warning, CROSS_ENV_WARNING);
        assert_eq!(pooled.envs.len(), 2);
        assert_eq!(pooled.result.successes, 1);
        assert_eq!(pooled.result.failures, 1);
        assert!(close(pooled.result.asr, 0.5));
    }

    #[test]
    fn cross_instrument_mean_requires_marker_and_stamps_warning() {
        let i1 = inst("rubric", "v1");
        let i2 = inst("rubric", "v2");
        // v1 は 1.0，v2 は 0.0
        let trials = vec![
            trial("a", 1, AttackRef::identity(), i1.clone(), Verdict::Success),
            trial("a", 1, AttackRef::identity(), i2.clone(), Verdict::Failure),
        ];
        let by_env = BTreeMap::from([(EnvKind::StaticPrompt, trials)]);
        let out = single_shot_asr_by_env(&by_env, Z_95);
        let env_asr = &out[&EnvKind::StaticPrompt];
        assert_eq!(env_asr.per_instrument.len(), 2);
        let mean = mean_asr_across_instruments(
            env_asr,
            CrossInstrument::i_understand_instruments_differ(),
        );
        assert_eq!(mean.warning, CROSS_INSTRUMENT_WARNING);
        assert!(close(mean.mean_asr, 0.5), "mean={}", mean.mean_asr);
    }

    #[test]
    fn results_serde_roundtrip() {
        let ci = wilson_interval(50, 100, Z_95);
        let json = serde_json::to_string(&ci).unwrap();
        let back: ConfidenceInterval = serde_json::from_str(&json).unwrap();
        assert_eq!(ci, back);
    }
}
