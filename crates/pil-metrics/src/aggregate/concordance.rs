//! `concordance` — Kendall の一致係数 W（DESIGN §3.7 / §8.1）．
//!
//! §3.7 の核心は「結論がどの判定器（judge）に依存するか」である．StrongREJECT v1/v2 の
//! ように測定器を替えると Case の順位付けが動くとき，複数測定器の順位がどれだけ一致して
//! いるかを 1 つの尺度で表すのが Kendall の一致係数 W である（W∈[0,1]，1 が完全一致）．
//!
//! 本モジュールは **network-free**．既知のテストベクタ（手計算）で検証する．W は集計側の
//! 「異なる測定器の数字を同じ土俵に載せる（§3.7）」の危険を，比較不能性の度合いとして
//! 開示するために使う（`pil-report` の cross-env 開示に同梱する）．
//!
//! 集計は tie-corrected（同順位補正つき）で行う（§11.3 に整合する保守的な扱い）：
//!
//! - m = 評価者（rater）数，n = 項目（item）数．
//! - 各評価者のスコアを **同順位は平均順位**で順位へ変換する（スコアが高いほど順位値も高い）．
//! - R_i = Σ_rater rank(item i)，R̄ = m(n+1)/2，S = Σ_i (R_i − R̄)²．
//! - 同順位補正 T_j = Σ_k (t_k³ − t_k)（評価者 j の同順位群サイズ t_k）．
//! - **W = 12·S / ( m²·(n³ − n) − m·Σ_j T_j )**．
//! - n<2 または m<2 は未定義（`None`）．完全同順位で分母 0 も `None`．

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use pil_core::{CaseId, InstrumentRef, Trial, Verdict};

use super::distinct_instruments;

/// Kendall の一致係数 W の結果（DESIGN §3.7）．
///
/// `w` は同順位補正済みの一致係数（W∈[0,1]）．`chi_square = m·(n−1)·W` は大標本近似で
/// あり，**n が小さいときは有効でない**（自由度 `df = n−1` の χ² 近似）．小 n では W 自体で判断する．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KendallW {
    /// 一致係数（同順位補正済み，W∈[0,1]）．
    pub w: f64,
    /// 項目数 n（順位付けの対象）．
    pub n_items: usize,
    /// 評価者数 m（順位を付ける主体）．
    pub m_raters: usize,
    /// 順位和の平均まわりの平方和 S．
    pub s: f64,
    /// 大標本 χ² 近似 `m·(n−1)·W`（小 n では無効）．
    pub chi_square: f64,
    /// χ² 近似の自由度 `n−1`．
    pub df: usize,
}

/// スコア列を平均順位（同順位補正）へ変換し，同順位補正項 `T = Σ(t³−t)` を返す．
///
/// スコアが高いほど順位値も高い（昇順ソートの位置＝順位）．W は順位付けの向きに不変なので，
/// 昇順・降順のどちらで数えても値は変わらない（S は平均まわりで対称）．
fn scores_to_ranks(scores: &[f64]) -> (Vec<f64>, f64) {
    let n = scores.len();
    // 昇順ソートした元インデックス列（同値は元順序を保つ安定ソート）．
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        scores[a]
            .partial_cmp(&scores[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut ranks = vec![0.0f64; n];
    let mut tie_correction = 0.0f64;
    let mut i = 0usize;
    while i < n {
        // [i, j) が同値群（1 始まり位置 i+1..=j）．
        let mut j = i + 1;
        while j < n && scores[order[j]] == scores[order[i]] {
            j += 1;
        }
        let group = j - i;
        // 位置 i+1..=j の平均 = ((i+1) + j) / 2．
        let avg_rank = ((i + 1) + j) as f64 / 2.0;
        for &idx in &order[i..j] {
            ranks[idx] = avg_rank;
        }
        if group > 1 {
            let t = group as f64;
            tie_correction += t * t * t - t;
        }
        i = j;
    }
    (ranks, tie_correction)
}

/// 同順位補正つき Kendall の一致係数 W を計算する（DESIGN §3.7）．
///
/// `rankings[r]` は評価者 r の**スコア列**（全評価者で同じ n 項目・添字整合，スコアが高いほど
/// 上位）．内部で各評価者のスコアを平均順位へ変換して W を出す．
///
/// `None` を返す条件：評価者 m<2，項目 n<2，評価者間で項目数が食い違う，または分母が 0 以下
/// （全項目が完全同順位で一致度が未定義）．
pub fn kendall_w(rankings: &[Vec<f64>]) -> Option<KendallW> {
    let m = rankings.len();
    if m < 2 {
        return None;
    }
    let n = rankings[0].len();
    if n < 2 {
        return None;
    }
    if rankings.iter().any(|r| r.len() != n) {
        return None;
    }

    let mut col_sums = vec![0.0f64; n];
    let mut sum_t = 0.0f64;
    for scores in rankings {
        let (ranks, t) = scores_to_ranks(scores);
        sum_t += t;
        for (i, r) in ranks.iter().enumerate() {
            col_sums[i] += r;
        }
    }

    let m_f = m as f64;
    let n_f = n as f64;
    let rbar = m_f * (n_f + 1.0) / 2.0;
    let s: f64 = col_sums.iter().map(|r| (r - rbar).powi(2)).sum();
    let denom = m_f * m_f * (n_f * n_f * n_f - n_f) - m_f * sum_t;
    if denom <= 0.0 {
        // 全評価者が全項目を同順位にした等，一致度が未定義（§3.7）．
        return None;
    }
    let w = 12.0 * s / denom;
    Some(KendallW {
        w,
        n_items: n,
        m_raters: m,
        s,
        chi_square: m_f * (n_f - 1.0) * w,
        df: n - 1,
    })
}

/// 測定器跨ぎの一致度（DESIGN §3.7）．評価者=測定器・項目=Case で Kendall W を出す．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentConcordance {
    /// 測定器を評価者・Case を項目とした一致係数 W．
    pub kendall: KendallW,
    /// 一致度を測った測定器の一覧（順位ベクタの行に対応）．
    pub instruments: Vec<InstrumentRef>,
    /// 実際に W の計算に使った Case 数（全測定器が判定可能だった共通 Case）．
    pub n_cases_used: usize,
    /// 共通 Case のうち，いずれかの測定器が `Undecidable` で除外した件数（§5.3）．
    pub n_cases_dropped_undecidable: usize,
}

/// 測定器跨ぎの Kendall W を，共通 Case 上で計算する（DESIGN §3.7）．
///
/// 手順：
/// 1. 測定器を [`distinct_instruments`] で出現順に一意化する（2 未満なら `None`）．
/// 2. Case ごとに (測定器 → 成功/失敗/判定不能 件数) を集計する．
/// 3. **全測定器が測定した共通 Case（交差）**だけを対象にする．
/// 4. 共通 Case のうち，**いずれかの測定器が `Undecidable` を含む Case は除外**し件数を保持する（§5.3）．
/// 5. 残った Case で，各測定器のスコア列（成功=1.0，失敗=0.0；同一 Case を複数回測ったら平均）を作り，
///    [`kendall_w`] を当てる．共通 Case が 2 未満なら `None`．
///
/// これは §3.7「順位付けはどの judge に依存するか」を，既存の多測定器データからそのまま実現する．
pub fn instrument_concordance(trials: &[Trial]) -> Option<InstrumentConcordance> {
    let instruments = distinct_instruments(trials);
    if instruments.len() < 2 {
        return None;
    }
    let k = instruments.len();

    // CaseId -> 測定器ごとの (successes, failures, undecidable)．CaseId は Ord なので決定論的順序．
    let mut by_case: BTreeMap<CaseId, Vec<(usize, usize, usize)>> = BTreeMap::new();
    for t in trials {
        for meas in &t.measurements {
            let Some(idx) = instruments.iter().position(|i| i == &meas.instrument) else {
                continue;
            };
            let slot = by_case
                .entry(t.case.clone())
                .or_insert_with(|| vec![(0, 0, 0); k]);
            match &meas.verdict {
                Verdict::Success => slot[idx].0 += 1,
                Verdict::Failure => slot[idx].1 += 1,
                Verdict::Undecidable { .. } => slot[idx].2 += 1,
            }
        }
    }

    // 各測定器のスコア列（共通 Case・Undecidable 除外後の順で index 整合）．
    let mut rankings: Vec<Vec<f64>> = vec![Vec::new(); k];
    let mut n_cases_used = 0usize;
    let mut n_cases_dropped_undecidable = 0usize;

    for tallies in by_case.values() {
        // 全測定器が測定したか（交差）．
        let measured_by_all = tallies.iter().all(|(s, f, u)| s + f + u > 0);
        if !measured_by_all {
            continue;
        }
        // いずれかの測定器で Undecidable を含む Case は除外（§5.3）．
        let any_undecidable = tallies.iter().any(|(_, _, u)| *u > 0);
        if any_undecidable {
            n_cases_dropped_undecidable += 1;
            continue;
        }
        for (idx, (s, f, _)) in tallies.iter().enumerate() {
            let decided = s + f;
            // Undecidable を除外済みなので decided>0 が保証される．
            let score = *s as f64 / decided as f64;
            rankings[idx].push(score);
        }
        n_cases_used += 1;
    }

    if n_cases_used < 2 {
        return None;
    }
    let kendall = kendall_w(&rankings)?;
    Some(InstrumentConcordance {
        kendall,
        instruments,
        n_cases_used,
        n_cases_dropped_undecidable,
    })
}

/// 連続スコアに基づく測定器一致度（DESIGN §3.7）．
///
/// [`InstrumentConcordance`] が二値 [`Verdict`] を評価者間の順位に載せるのに対し，本型は
/// `Measurement.score`（例: StrongREJECT の `[0,1]` 連続スコア）そのものを順位化して
/// [`kendall_w`] に載せる．`group` は全選択測定器のグループ一致係数，`pairwise` は測定器の
/// 各対に対する一致係数（m=2 の Kendall W は `[0,1]` の一致度として妥当）である．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreConcordance {
    /// 全選択測定器を評価者とするグループ一致係数 W．
    pub group: KendallW,
    /// 測定器の各対 `(a, b)` に対する一致係数 W（m=2）．退化（分母 0）の対は除外する．
    pub pairwise: Vec<(InstrumentRef, InstrumentRef, KendallW)>,
    /// 一致度を測った測定器の一覧（順位ベクタの行に対応）．
    pub instruments: Vec<InstrumentRef>,
    /// 実際に W の計算に使った共通 Case 数（全測定器がスコアを持った Case）．
    pub n_cases_used: usize,
    /// 共通 Case のうち，いずれかの選択測定器が `Undecidable` かスコア欠落で除外した件数（§5.3）．
    pub n_cases_dropped: usize,
}

/// 連続スコアに基づく測定器跨ぎ Kendall W を，共通 Case 上で計算する（DESIGN §3.7）．
///
/// [`instrument_concordance`] の二値版に対する**スコア版**の兄弟関数である．手順：
///
/// 1. `trials` に現れる測定器を [`distinct_instruments`] 相当の線形 dedup で一意化し，`select` を
///    満たすものだけを選ぶ（2 未満なら `None`）．
/// 2. Case ごとに，選択測定器それぞれの `Measurement.score` を集める（同一 Case を同一測定器で
///    複数回測ったら**平均**を採る）．
/// 3. **全選択測定器が測定した共通 Case（交差）**だけを対象にする．
/// 4. 共通 Case のうち，いずれかの選択測定器の測定が `Undecidable` **または** `score == None` の
///    Case は除外し件数を `n_cases_dropped` に保持する（スコアを持つ判定器のみ；§5.3）．
/// 5. 残った Case で各測定器のスコア列（index 整合）を作り，[`kendall_w`] に載せてグループ W を出す．
///    さらに測定器の各対について，その 2 本のスコア列だけを [`kendall_w`] に渡してペア別 W を出す．
///
/// `None` を返す条件：選択測定器が 2 未満，残った共通 Case が 2 未満，またはグループ W が未定義
/// （全測定器が全 Case を同順位）．
pub fn score_concordance(
    trials: &[Trial],
    select: impl Fn(&InstrumentRef) -> bool,
) -> Option<ScoreConcordance> {
    // distinct_instruments 相当の線形 dedup（InstrumentRef は Ord/Hash を持たない）で選択する．
    let mut instruments: Vec<InstrumentRef> = Vec::new();
    for t in trials {
        for m in &t.measurements {
            if select(&m.instrument) && !instruments.iter().any(|i| i == &m.instrument) {
                instruments.push(m.instrument.clone());
            }
        }
    }
    if instruments.len() < 2 {
        return None;
    }
    let k = instruments.len();

    // 選択測定器ごとの Case 内集計．measured=測定の有無（交差判定），bad=Undecidable かスコア欠落．
    #[derive(Clone)]
    struct Cell {
        measured: bool,
        bad: bool,
        sum: f64,
        count: usize,
    }
    let empty = Cell {
        measured: false,
        bad: false,
        sum: 0.0,
        count: 0,
    };

    // CaseId は Ord なので BTreeMap で決定論的順序．
    let mut by_case: BTreeMap<CaseId, Vec<Cell>> = BTreeMap::new();
    for t in trials {
        for meas in &t.measurements {
            let Some(idx) = instruments.iter().position(|i| i == &meas.instrument) else {
                continue;
            };
            let cell = &mut by_case
                .entry(t.case.clone())
                .or_insert_with(|| vec![empty.clone(); k])[idx];
            cell.measured = true;
            match (&meas.verdict, meas.score) {
                // スコアを持つ判定（成功/失敗いずれでも連続スコアを使う）．
                (Verdict::Success | Verdict::Failure, Some(s)) => {
                    cell.sum += s;
                    cell.count += 1;
                }
                // Undecidable もしくはスコア欠落は除外対象（三値の規律，§5.3）．
                _ => cell.bad = true,
            }
        }
    }

    let mut rankings: Vec<Vec<f64>> = vec![Vec::new(); k];
    let mut n_cases_used = 0usize;
    let mut n_cases_dropped = 0usize;

    for cells in by_case.values() {
        // 全選択測定器が測定したか（交差）．
        if !cells.iter().all(|c| c.measured) {
            continue;
        }
        // いずれかが Undecidable/スコア欠落なら除外し件数を保持する（§5.3）．
        if cells.iter().any(|c| c.bad) {
            n_cases_dropped += 1;
            continue;
        }
        for (idx, c) in cells.iter().enumerate() {
            // measured かつ !bad なので count>=1 が保証される．
            rankings[idx].push(c.sum / c.count as f64);
        }
        n_cases_used += 1;
    }

    if n_cases_used < 2 {
        return None;
    }

    let group = kendall_w(&rankings)?;

    // ペア別一致度：各非順序対のスコア列 2 本だけで Kendall W を出す（退化した対は除外）．
    let mut pairwise: Vec<(InstrumentRef, InstrumentRef, KendallW)> = Vec::new();
    for a in 0..k {
        for b in (a + 1)..k {
            if let Some(w) = kendall_w(&[rankings[a].clone(), rankings[b].clone()]) {
                pairwise.push((instruments[a].clone(), instruments[b].clone(), w));
            }
        }
    }

    Some(ScoreConcordance {
        group,
        pairwise,
        instruments,
        n_cases_used,
        n_cases_dropped,
    })
}

/// StrongREJECT 3 判定器（rubric v1 / rubric v2 / fine-tuned）の連続スコア一致度（DESIGN §3.7）．
///
/// §3.7 の「StrongREJECT の judge は 2 つある」という懸念を**3 つ**へ拡張し，StrongREJECT スコアが
/// どの実装（judge）に依存するかを定量化する．選択条件は測定器名の接頭辞 `"strongreject"`
/// （`strongreject-rubric` / `strongreject-finetuned` が該当）．2 種未満しか無ければ `None`．
pub fn strongreject_score_concordance(trials: &[Trial]) -> Option<ScoreConcordance> {
    score_concordance(trials, |i| i.name.starts_with("strongreject"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_core::{
        AttackRef, FinishReason, Measurement, MeasurementParams, ModelRef, Response, SourceRef,
        UndecidableReason,
    };

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // --- kendall_w: 手計算のテストベクタ ---

    /// (a) 全評価者が同一順位 → W = 1.0（完全一致）．
    /// 3 評価者・4 項目，スコア [1,2,3,4]．R_i=[3,6,9,12]，R̄=7.5，S=45，分母=9·60=540，W=540/540=1．
    #[test]
    fn kendall_w_perfect_concordance_is_one() {
        let r = vec![
            vec![1.0, 2.0, 3.0, 4.0],
            vec![1.0, 2.0, 3.0, 4.0],
            vec![1.0, 2.0, 3.0, 4.0],
        ];
        let k = kendall_w(&r).expect("defined");
        assert_eq!(k.w, 1.0);
        assert_eq!(k.n_items, 4);
        assert_eq!(k.m_raters, 3);
        assert!(close(k.s, 45.0), "S={}", k.s);
        assert_eq!(k.df, 3);
        // χ² = m(n-1)W = 3·3·1 = 9
        assert!(close(k.chi_square, 9.0), "chi={}", k.chi_square);
    }

    /// (b) 教科書例：3 評価者・4 項目（同順位なし）．
    /// ranks r1=[1,2,3,4], r2=[2,1,3,4], r3=[1,2,3,4]．
    /// 列和=[4,5,9,12]，R̄=7.5，S=41，分母=540，W=492/540=0.9111…．
    #[test]
    fn kendall_w_textbook_no_ties() {
        let r = vec![
            vec![1.0, 2.0, 3.0, 4.0],
            vec![2.0, 1.0, 3.0, 4.0],
            vec![1.0, 2.0, 3.0, 4.0],
        ];
        let k = kendall_w(&r).expect("defined");
        assert!(close(k.s, 41.0), "S={}", k.s);
        assert!(close(k.w, 492.0 / 540.0), "W={}", k.w);
        assert!(close(k.w, 0.911_111_111_111_111), "W={}", k.w);
    }

    /// (c) 同順位を含む：2 評価者・4 項目．
    /// r1 スコア [10,10,20,30] → ranks [1.5,1.5,3,4]（同順位群サイズ2，T=6），r2 [5,6,7,8] → [1,2,3,4]．
    /// 列和=[2.5,3.5,6,8]，R̄=5，S=18.5，分母=4·60−2·6=228，W=222/228=0.973684…．
    #[test]
    fn kendall_w_with_ties_uses_correction() {
        let r = vec![vec![10.0, 10.0, 20.0, 30.0], vec![5.0, 6.0, 7.0, 8.0]];
        let k = kendall_w(&r).expect("defined");
        assert!(close(k.s, 18.5), "S={}", k.s);
        assert!(close(k.w, 222.0 / 228.0), "W={}", k.w);
        assert!(close(k.w, 0.973_684_210_526_315_8), "W={}", k.w);
    }

    /// (d) m<2 または n<2 → None．
    #[test]
    fn kendall_w_undefined_for_small_inputs() {
        assert!(kendall_w(&[vec![1.0, 2.0, 3.0]]).is_none()); // m=1
        assert!(kendall_w(&[vec![1.0], vec![2.0]]).is_none()); // n=1
        assert!(kendall_w(&[]).is_none()); // m=0
    }

    /// 全項目・全評価者が完全同順位 → 分母 0 で未定義（None）．
    #[test]
    fn kendall_w_all_tied_is_none() {
        let r = vec![vec![1.0, 1.0, 1.0], vec![2.0, 2.0, 2.0]];
        assert!(kendall_w(&r).is_none());
    }

    // --- instrument_concordance: 合成 Trial ---

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

    /// 1 Case を 2 測定器で測る Trial を作る．
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

    /// 2 測定器が一致 → W が高い（同一スコア列は W=1.0）．
    #[test]
    fn instrument_concordance_agreement_is_high() {
        let a = inst("rubric", "v1");
        let b = inst("rubric", "v2");
        let trials = vec![
            trial_two("c1", &a, Verdict::Success, &b, Verdict::Success),
            trial_two("c2", &a, Verdict::Failure, &b, Verdict::Failure),
            trial_two("c3", &a, Verdict::Success, &b, Verdict::Success),
        ];
        let ic = instrument_concordance(&trials).expect("defined");
        assert_eq!(ic.n_cases_used, 3);
        assert_eq!(ic.n_cases_dropped_undecidable, 0);
        assert_eq!(ic.instruments.len(), 2);
        assert_eq!(ic.kendall.w, 1.0);
    }

    /// 2 測定器が不一致 → W が低い．
    /// A=[1,1,0], B=[0,1,1] → 各 ranks で W=18/72=0.25．
    #[test]
    fn instrument_concordance_disagreement_is_low() {
        let a = inst("rubric", "v1");
        let b = inst("rubric", "v2");
        let trials = vec![
            trial_two("c1", &a, Verdict::Success, &b, Verdict::Failure),
            trial_two("c2", &a, Verdict::Success, &b, Verdict::Success),
            trial_two("c3", &a, Verdict::Failure, &b, Verdict::Success),
        ];
        let ic = instrument_concordance(&trials).expect("defined");
        assert_eq!(ic.n_cases_used, 3);
        assert!(close(ic.kendall.w, 0.25), "W={}", ic.kendall.w);
    }

    /// いずれかの測定器が Undecidable の Case は除外し件数を保持する（§5.3）．
    #[test]
    fn instrument_concordance_excludes_and_counts_undecidable() {
        let a = inst("rubric", "v1");
        let b = inst("rubric", "v2");
        let trials = vec![
            trial_two("c1", &a, Verdict::Success, &b, Verdict::Success),
            trial_two("c2", &a, Verdict::Failure, &b, Verdict::Failure),
            // c3 は b が判定不能 → 除外され dropped=1
            trial_two("c3", &a, Verdict::Success, &b, undec()),
        ];
        let ic = instrument_concordance(&trials).expect("defined");
        assert_eq!(ic.n_cases_used, 2);
        assert_eq!(ic.n_cases_dropped_undecidable, 1);
    }

    /// 測定器が 1 種のみ → None．
    #[test]
    fn instrument_concordance_needs_two_instruments() {
        let a = inst("rubric", "v1");
        let trials = vec![Trial {
            case: cid("c1"),
            attempt: 1,
            model: ModelRef::new("ollama", "m", None),
            attack: AttackRef::identity(),
            response: resp(),
            measurements: vec![Measurement {
                verdict: Verdict::Success,
                score: None,
                instrument: a,
                raw: String::new(),
            }],
        }];
        assert!(instrument_concordance(&trials).is_none());
    }

    #[test]
    fn kendall_w_serde_roundtrip() {
        let k = kendall_w(&[vec![1.0, 2.0, 3.0], vec![1.0, 2.0, 3.0]]).unwrap();
        let json = serde_json::to_string(&k).unwrap();
        let back: KendallW = serde_json::from_str(&json).unwrap();
        assert_eq!(k, back);
    }

    // --- score_concordance: 連続スコアの合成 Trial ---

    /// 1 Case を，指定した (測定器, verdict, score) 群で測る Trial を作る．
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

    /// スコア付き成功測定を作る簡便関数．
    fn scored(score: f64) -> (Verdict, Option<f64>) {
        (Verdict::Success, Some(score))
    }

    fn find_pair<'a>(
        sc: &'a ScoreConcordance,
        a: &InstrumentRef,
        b: &InstrumentRef,
    ) -> &'a KendallW {
        sc.pairwise
            .iter()
            .find_map(|(x, y, w)| {
                if (x == a && y == b) || (x == b && y == a) {
                    Some(w)
                } else {
                    None
                }
            })
            .expect("pair present")
    }

    /// (a) 3 判定器が同一のスコア順位 → グループ W=1.0 かつ全ペア W=1.0．
    #[test]
    fn score_concordance_identical_rankings_is_one() {
        let v1 = inst("strongreject-rubric", "v1");
        let v2 = inst("strongreject-rubric", "v2");
        let ft = inst("strongreject-finetuned", "15k-v1");
        let scores = [0.1, 0.4, 0.6, 0.9];
        let trials: Vec<Trial> = scores
            .iter()
            .enumerate()
            .map(|(idx, &s)| {
                let (sv, ss) = scored(s);
                trial_scored(
                    &format!("c{idx}"),
                    &[(&v1, sv.clone(), ss), (&v2, sv.clone(), ss), (&ft, sv, ss)],
                )
            })
            .collect();
        let sc = strongreject_score_concordance(&trials).expect("defined");
        assert_eq!(sc.instruments.len(), 3);
        assert_eq!(sc.n_cases_used, 4);
        assert_eq!(sc.n_cases_dropped, 0);
        assert_eq!(sc.group.w, 1.0);
        assert_eq!(sc.pairwise.len(), 3);
        for (_, _, w) in &sc.pairwise {
            assert_eq!(w.w, 1.0);
        }
    }

    /// (b) 1 判定器が順位を反転 → グループ W が下がる（手計算値）．
    /// v1,v2 昇順 [.1,.2,.3,.4]，ft 降順 [.4,.3,.2,.1]．列和=[6,7,8,9]，S=5，分母=540，W=60/540=1/9．
    /// ペア: v1↔v2=1.0，v1↔ft=0.0，v2↔ft=0.0．
    #[test]
    fn score_concordance_one_inverts_lowers_group_w() {
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
        let sc = strongreject_score_concordance(&trials).expect("defined");
        assert_eq!(sc.n_cases_used, 4);
        assert!(close(sc.group.w, 1.0 / 9.0), "group W={}", sc.group.w);
        assert!(close(find_pair(&sc, &v1, &v2).w, 1.0));
        assert!(close(find_pair(&sc, &v1, &ft).w, 0.0));
        assert!(close(find_pair(&sc, &v2, &ft).w, 0.0));
    }

    /// (c) 1 判定器が Undecidable/スコア欠落の Case は除外し n_cases_dropped=1．
    #[test]
    fn score_concordance_drops_undecidable_or_scoreless_case() {
        let v1 = inst("strongreject-rubric", "v1");
        let v2 = inst("strongreject-rubric", "v2");
        let ft = inst("strongreject-finetuned", "15k-v1");
        let good = |c: &str, s: f64| {
            trial_scored(
                c,
                &[
                    (&v1, Verdict::Success, Some(s)),
                    (&v2, Verdict::Success, Some(s)),
                    (&ft, Verdict::Success, Some(s)),
                ],
            )
        };
        let trials = vec![
            good("c1", 0.2),
            good("c2", 0.8),
            // c3: ft が Undecidable（スコア None）→ 除外
            trial_scored(
                "c3",
                &[
                    (&v1, Verdict::Success, Some(0.5)),
                    (&v2, Verdict::Success, Some(0.5)),
                    (&ft, undec(), None),
                ],
            ),
        ];
        let sc = strongreject_score_concordance(&trials).expect("defined");
        assert_eq!(sc.n_cases_used, 2);
        assert_eq!(sc.n_cases_dropped, 1);
    }

    /// (d) StrongREJECT 判定器が 2 種未満 → None．
    #[test]
    fn score_concordance_needs_two_sr_judges() {
        let v1 = inst("strongreject-rubric", "v1");
        let trials = vec![
            trial_scored("c1", &[(&v1, Verdict::Success, Some(0.2))]),
            trial_scored("c2", &[(&v1, Verdict::Success, Some(0.8))]),
        ];
        assert!(strongreject_score_concordance(&trials).is_none());
    }

    /// 非 StrongREJECT 測定器は選択されない（instruments に含まれない）．
    #[test]
    fn score_concordance_excludes_non_sr_instrument() {
        let v1 = inst("strongreject-rubric", "v1");
        let v2 = inst("strongreject-rubric", "v2");
        let other = inst("string-match", "v1");
        let trials: Vec<Trial> = (0..3)
            .map(|idx| {
                let s = 0.2 + 0.3 * idx as f64;
                trial_scored(
                    &format!("c{idx}"),
                    &[
                        (&v1, Verdict::Success, Some(s)),
                        (&v2, Verdict::Success, Some(s)),
                        (&other, Verdict::Success, Some(1.0 - s)),
                    ],
                )
            })
            .collect();
        let sc = strongreject_score_concordance(&trials).expect("defined");
        assert_eq!(sc.instruments.len(), 2);
        assert!(sc
            .instruments
            .iter()
            .all(|i| i.name.starts_with("strongreject")));
    }

    #[test]
    fn score_concordance_serde_roundtrip() {
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
        let sc = strongreject_score_concordance(&trials).expect("defined");
        let json = serde_json::to_string(&sc).unwrap();
        let back: ScoreConcordance = serde_json::from_str(&json).unwrap();
        assert_eq!(sc.instruments, back.instruments);
        assert_eq!(sc.n_cases_used, back.n_cases_used);
        assert!((sc.group.w - back.group.w).abs() < 1e-12);
    }
}
