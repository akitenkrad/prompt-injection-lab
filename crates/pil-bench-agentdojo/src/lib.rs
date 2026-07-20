//! `pil-bench-agentdojo` — AgentDojo 取り込みの native-first アダプタ（DESIGN §4.1 / §4.2 / §10）．
//!
//! AgentDojo v0.1.35（submodule `third_party/agentdojo`，pin [`COMMIT`]，benchmark_version
//! [`BENCHMARK_VERSION`]）を対象とする．AgentDojo の環境・ツール・scoring は完全 in-memory の
//! irreducible な Python 本体であり，Rust へ移植すると「同一ベンチ」でなくなる（§4.1）．そこで本 crate は
//! 「測定値を変えない」層のみを Rust に集約する:
//!
//! - **ケース型・provenance**（[`AgentDojoCase`] → [`pil_core::Case`]，`EnvKind::Emulated`，[`SourceRef`]）．
//! - **結果 JSON パース**（[`AgentDojoResult`]，AgentDojo が永続化する `TaskResults` 形状）と，
//!   注入次元の [`Verdict`] への正規化（[`AgentDojoResult::verdict`]）．
//! - **列挙 ingest**（Python 列挙器の JSON → `Vec<Case>`，[`load_enumeration`]）．
//!
//! ケース列挙・単一ケース実行（`python/enumerate_cases.py` / `python/run_case.py`）は agentdojo の
//! pip インストールを要する薄殻であり，既定テストでは動かさない．完全な end-to-end ライブ実行は
//! network-free ではない（シムの tool-calling 対応・agentdojo 実インストール・実ツール対応モデルが要る）．
//! そのため sidecar 駆動のライブ経路は `agentdojo` feature ＋ `#[ignore]` の統合テストとして文書化し，
//! 既定 CI から外す（§4.1 の network-free 検証可能スライス）．
//!
//! **注意**: adapter 種別（sidecar/native）は実装都合であり，`EnvKind`（科学的性質）とは別物である（§4.2）．

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use pil_core::{Case, InstrumentRef, Measurement, MeasurementParams, SourceRef};

// アダプタ利用側（統合テスト・runner）が pil-core を直接依存に足さず判定型へ触れられるよう再輸出する．
pub use pil_core::{EnvKind, UndecidableReason, Verdict};

/// 上流リポジトリ識別子（DESIGN §7.1 / §4.1）．
pub const UPSTREAM: &str = "ethz-spylab/agentdojo";
/// 固定 SHA（フル）．submodule `third_party/agentdojo` の pin（tag v0.1.35）と一致する．
pub const COMMIT: &str = "a75aba7631d3ca5fb7ab938965c97ead2f9ff84b";
/// AgentDojo の benchmark_version．`get_suites(...)` のタスク version フィルタに用いる値．
pub const BENCHMARK_VERSION: &str = "v1.2.2";
/// AgentDojo の 4 suite（DESIGN §4.1 調査サマリ）．
pub const SUITES: [&str; 4] = ["banking", "slack", "travel", "workspace"];
/// 注入次元の判定を与える AgentDojo 本体のファイル（provenance 用）．
const SECURITY_PREDICATE_PATH: &str = "src/agentdojo/benchmark.py";

/// アダプタのエラー（DESIGN §4.1）．
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// 列挙 JSON / 結果 JSON のデコードに失敗した（スキーマ不整合）．
    #[error("AgentDojo JSON のデコードに失敗しました: {0}")]
    Json(#[from] serde_json::Error),
}

/// AgentDojo の 1 security ケースの同一性材料（DESIGN §5.2 / §4.1）．
///
/// security ケースは `(suite, user_task, injection_task)` の組で決まる（`security=True` は注入成功）．
/// 実タスク文は Python 本体（in-memory 環境）側にしか無いため，network-free では組を identity として
/// 扱い，provenance を [`SourceRef`] に，識別文字列を prompt/context に写す．
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDojoCase {
    /// suite 名（`banking` / `slack` / `travel` / `workspace`）．
    pub suite: String,
    /// user task の ID（例 `user_task_0`）．
    pub user_task_id: String,
    /// injection task の ID（例 `injection_task_1`）．security ケースでは必須．
    pub injection_task_id: String,
}

impl AgentDojoCase {
    /// 素の組から構築する．
    pub fn new(
        suite: impl Into<String>,
        user_task_id: impl Into<String>,
        injection_task_id: impl Into<String>,
    ) -> Self {
        Self {
            suite: suite.into(),
            user_task_id: user_task_id.into(),
            injection_task_id: injection_task_id.into(),
        }
    }

    /// この security ケースの provenance（DESIGN §5.1）．
    ///
    /// `path` に `<suite>/<user_task_id> x <injection_task_id>` を写し，AgentDojo に CSV 行の概念が
    /// 無いため `row` は 0 に固定する（同一性は upstream/commit/path が担い，列挙順に依存しない）．
    pub fn source_ref(&self) -> SourceRef {
        SourceRef::new(
            UPSTREAM,
            COMMIT,
            format!("{} x {}", self.path_prefix(), self.injection_task_id),
            0,
        )
    }

    /// `<suite>/<user_task_id>`．
    fn path_prefix(&self) -> String {
        format!("{}/{}", self.suite, self.user_task_id)
    }

    /// `pil_core::Case` へ写す（`EnvKind::Emulated`，DESIGN §5.2 / §4.2）．
    ///
    /// - `prompt`＝`agentdojo/<suite>/<user_task_id>`（user task 次元）．
    /// - `context`＝`Some(injection_task_id)`（注入次元；`ContentKey` は §3.5 より context を含める）．
    /// - `target`＝`None`（肯定接頭辞の概念は無い）．`benign`＝`false`（security ケース）．
    /// - `labels` に suite/user/injection/benchmark_version を保持する（同一性の根拠にはしない，§3.3）．
    pub fn to_case(&self) -> Case {
        let mut labels = BTreeMap::new();
        labels.insert("suite".to_string(), self.suite.clone());
        labels.insert("user_task_id".to_string(), self.user_task_id.clone());
        labels.insert(
            "injection_task_id".to_string(),
            self.injection_task_id.clone(),
        );
        labels.insert(
            "benchmark_version".to_string(),
            BENCHMARK_VERSION.to_string(),
        );

        Case::new(
            self.source_ref(),
            format!("agentdojo/{}", self.path_prefix()),
            None,
            Some(self.injection_task_id.clone()),
            EnvKind::Emulated,
            false,
            labels,
        )
    }
}

/// 列挙器 JSON（`[{suite, user_task_id, injection_task_id}, ...]`）を `Vec<AgentDojoCase>` へ．
pub fn parse_enumeration(json: &str) -> Result<Vec<AgentDojoCase>, AdapterError> {
    Ok(serde_json::from_str(json)?)
}

/// 列挙器 JSON を直接 `Vec<Case>`（`EnvKind::Emulated`）へ写す ingest（DESIGN §4.1）．
pub fn load_enumeration(json: &str) -> Result<Vec<Case>, AdapterError> {
    Ok(parse_enumeration(json)?
        .iter()
        .map(AgentDojoCase::to_case)
        .collect())
}

/// AgentDojo が永続化する `TaskResults` JSON の形状（DESIGN §4.1 調査サマリ）．
///
/// `agentdojo/src/agentdojo/logging.py` の `save()` が書く辞書に対応する．`messages` は判定に不要な
/// 大きな配列のため `serde_json::Value` で保持する（欠落も許容）．`injections` も辞書のまま保持する．
/// 決定論的述語（LLM judge 非依存）による `utility`/`security` を核とする．
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDojoResult {
    /// suite 名（完全な `TaskResults` JSON にのみ在る．最小 driver 出力では欠落＝空）．
    #[serde(default)]
    pub suite_name: String,
    /// pipeline 名（モデル + 防御の合成名．最小 driver 出力では欠落＝空）．
    #[serde(default)]
    pub pipeline_name: String,
    /// user task ID（最小 driver 出力では欠落＝空）．
    #[serde(default)]
    pub user_task_id: String,
    /// injection task ID（`none` ケースでは `null`）．
    #[serde(default)]
    pub injection_task_id: Option<String>,
    /// 攻撃名（`important_instructions` 等；未注入は `null`）．
    #[serde(default)]
    pub attack_type: Option<String>,
    /// 注入テキスト（`{injection_vector: text}` の辞書）．
    #[serde(default)]
    pub injections: serde_json::Value,
    /// 会話履歴．判定には使わないが provenance として保持する（欠落可）．
    #[serde(default)]
    pub messages: serde_json::Value,
    /// パイプライン実行中のエラー文（正常時は `null`）．
    #[serde(default)]
    pub error: Option<String>,
    /// user task を達成したか（有用性次元）．**注入判定に潰さず保持する**．
    pub utility: bool,
    /// 注入が成功したか（`True` は攻撃成功）．注入次元の判定の核（§10）．
    pub security: bool,
    /// 実行の所要秒数（provenance；再現性のため測定値には含めない）．
    #[serde(default)]
    pub duration: Option<f64>,
    /// 実行時刻（provenance）．
    #[serde(default)]
    pub evaluation_timestamp: Option<String>,
    /// 実行した agentdojo パッケージ版（provenance）．
    #[serde(default)]
    pub agentdojo_package_version: Option<String>,
}

impl AgentDojoResult {
    /// 注入次元の三値判定へ正規化する（DESIGN §5.3 / §10）．
    ///
    /// - `error` が非空 → `Undecidable{ProviderError}`（レート制限・タイムアウト・内部エラー等；
    ///   0 にも 1 にも潰さない，§3.6）．
    /// - `security == true` → `Success`（注入成功）．
    /// - `security == false` → `Failure`．
    ///
    /// `utility`（有用性）はここでは潰さない．メタデータとして [`AgentDojoResult::utility`] に残り，
    /// [`AgentDojoResult::to_measurement`] の `raw` にも写る．
    pub fn verdict(&self) -> Verdict {
        match self.error.as_deref() {
            Some(msg) if !msg.trim().is_empty() => Verdict::Undecidable {
                reason: UndecidableReason::ProviderError {
                    message: msg.to_string(),
                },
            },
            _ if self.security => Verdict::Success,
            _ => Verdict::Failure,
        }
    }

    /// 注入判定を [`Measurement`] へ写す（DESIGN §5.4 / §5.5）．
    ///
    /// AgentDojo の security 述語は決定論的（LLM judge 非依存）なので `judge_model = None`・
    /// `temperature = 0.0`．`score` は二値述語のため `None`．`raw` に `utility`/`security`/`error` を
    /// 写して有用性次元を保持する（§5.3 の「潰さない」）．
    pub fn to_measurement(&self) -> Measurement {
        let raw = serde_json::json!({
            "utility": self.utility,
            "security": self.security,
            "error": self.error,
            "attack_type": self.attack_type,
        })
        .to_string();
        Measurement {
            verdict: self.verdict(),
            score: None,
            instrument: agentdojo_security_instrument(),
            raw,
        }
    }
}

/// AgentDojo の注入 security 述語を表す測定器参照（DESIGN §5.4）．
///
/// 同名別実体を避けるため `version` に benchmark_version を写し，`source` に AgentDojo 本体の
/// commit/ファイルを写す．決定論的述語のため judge 系パラメータは持たない．
pub fn agentdojo_security_instrument() -> InstrumentRef {
    InstrumentRef {
        name: "agentdojo-security".to_string(),
        version: BENCHMARK_VERSION.to_string(),
        source: SourceRef::new(UPSTREAM, COMMIT, SECURITY_PREDICATE_PATH, 0),
        params: MeasurementParams {
            response_clip_tokens: None,
            judge_model: None,
            temperature: 0.0,
        },
    }
}

/// 単一ケース driver（`run_case.py`）の `{utility, security, error}` JSON を parse する（DESIGN §4.1）．
///
/// これは薄殻 driver の最小出力であり，永続化 `TaskResults` の部分集合．
pub fn parse_result(json: &str) -> Result<AgentDojoResult, AdapterError> {
    Ok(serde_json::from_str(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentDojoCase {
        AgentDojoCase::new("banking", "user_task_0", "injection_task_1")
    }

    #[test]
    fn commit_pin_is_full_sha() {
        assert_eq!(COMMIT.len(), 40, "フル SHA（40 hex 桁）");
        assert!(COMMIT.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn to_case_is_emulated_with_pinned_provenance() {
        let case = sample().to_case();
        assert_eq!(
            case.env_kind,
            EnvKind::Emulated,
            "AgentDojo は §4.2 の Emulated"
        );
        assert_eq!(case.source.upstream, UPSTREAM);
        assert_eq!(case.source.commit, COMMIT, "SourceRef.commit は pin SHA");
        assert_eq!(case.source.path, "banking/user_task_0 x injection_task_1");
        assert_eq!(case.source.row, 0);
        assert!(!case.benign);
        assert!(case.target.is_none());
        assert_eq!(case.context.as_deref(), Some("injection_task_1"));
        assert_eq!(
            case.labels.get("suite").map(String::as_str),
            Some("banking")
        );
        assert_eq!(
            case.labels.get("benchmark_version").map(String::as_str),
            Some(BENCHMARK_VERSION)
        );
    }

    #[test]
    fn caseid_is_deterministic() {
        // 同一組 → 同一 CaseId（決定論），列挙順に依存しない．
        let a = sample().to_case();
        let b = sample().to_case();
        assert_eq!(a.id, b.id);
        assert_eq!(a.id.short().len(), 16);
        assert_eq!(a.id.full().len(), 64);
    }

    #[test]
    fn distinct_cases_have_distinct_ids() {
        let base = sample().to_case();
        let other_user = AgentDojoCase::new("banking", "user_task_1", "injection_task_1").to_case();
        let other_inj = AgentDojoCase::new("banking", "user_task_0", "injection_task_2").to_case();
        let other_suite = AgentDojoCase::new("slack", "user_task_0", "injection_task_1").to_case();
        assert_ne!(base.id, other_user.id);
        assert_ne!(
            base.id, other_inj.id,
            "injection 次元は context 経由で identity に効く"
        );
        assert_ne!(base.id, other_suite.id);
    }

    #[test]
    fn verdict_security_true_is_success() {
        let r = result(true, true, None);
        assert_eq!(r.verdict(), Verdict::Success);
    }

    #[test]
    fn verdict_security_false_is_failure() {
        let r = result(true, false, None);
        assert_eq!(r.verdict(), Verdict::Failure);
    }

    #[test]
    fn verdict_error_is_undecidable() {
        let r = result(false, true, Some("rate limit".to_string()));
        match r.verdict() {
            Verdict::Undecidable {
                reason: UndecidableReason::ProviderError { message },
            } => assert_eq!(message, "rate limit"),
            other => panic!("expected ProviderError Undecidable, got {other:?}"),
        }
    }

    #[test]
    fn empty_error_string_does_not_force_undecidable() {
        // "" は「エラー無し」とみなす（§3.6: 潰す先を誤らない）．
        let r = result(true, true, Some("   ".to_string()));
        assert_eq!(r.verdict(), Verdict::Success);
    }

    #[test]
    fn measurement_preserves_utility() {
        let r = result(true, false, None);
        let m = r.to_measurement();
        assert_eq!(m.verdict, Verdict::Failure);
        assert!(m.score.is_none());
        assert_eq!(m.instrument.name, "agentdojo-security");
        assert!(
            m.raw.contains("\"utility\":true"),
            "utility を潰さず raw に保持"
        );
    }

    fn result(utility: bool, security: bool, error: Option<String>) -> AgentDojoResult {
        AgentDojoResult {
            suite_name: "banking".into(),
            pipeline_name: "gpt-4o".into(),
            user_task_id: "user_task_0".into(),
            injection_task_id: Some("injection_task_1".into()),
            attack_type: Some("important_instructions".into()),
            injections: serde_json::json!({}),
            messages: serde_json::Value::Null,
            error,
            utility,
            security,
            duration: Some(1.5),
            evaluation_timestamp: Some("2026-07-19 00:00:00".into()),
            agentdojo_package_version: Some("0.1.35".into()),
        }
    }
}
