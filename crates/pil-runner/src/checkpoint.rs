//! 中断再開のためのチェックポイント（DESIGN §6.2 / §11.3）．
//!
//! `(CaseId, instrument, attempt, seed)` 単位の **append-only JSONL** を耐久記録として残し，
//! 再起動時に完了タプルを読み込んでスキップする（§11.3）．本設計では union coverage の
//! バリアント軸（§5.6）を正しく扱うため，タプルに `attack` も含める — これは §6.2 の
//! キャッシュキーが `rendered_prompt`（＝変換適用後の最終プロンプト）を含むのと同じ理由で，
//! 同一 Case の異なる変換を別物として数えるためである．
//!
//! # 冪等性（DESIGN §11.3「追記は atomic write/rename で冪等」）
//!
//! 二重生成を防ぐため 2 段の防御を持つ:
//!
//! 1. **完了タプルガード**: 追記前に in-memory の完了集合を照合し，既存タプルは書かない．
//! 2. **1 生成 = 1 バッチ追記**: 1 回の生成に紐づく全測定タプルを**まとめて 1 度に追記**する．
//!    生成途中でのプロセス kill では「その生成のタプルが 1 件も存在しない」状態にしかならず，
//!    「一部だけ記録され再開時に再生成される」事態を避ける（生成の原子性）．
//!
//! さらに [`Checkpoint::compact`] は temp へ全書き出し → fsync → rename の atomic write/rename を
//! 提供する（§11.3）．

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use pil_core::{AttackRef, CaseId, Measurement, ModelRef, Response, Trial};

/// 完了タプルの同一性（DESIGN §11.3）．`(CaseId, instrument, attempt, seed)` に `attack` を足す．
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TupleKey {
    pub case: CaseId,
    pub attack: AttackRef,
    /// 測定器の name（`InstrumentRef.name`）
    pub instrument_name: String,
    /// 測定器の version（`InstrumentRef.version`）— v1/v2 を区別（§3.7 / §5.4）
    pub instrument_version: String,
    pub attempt: u32,
    /// 実送信 seed（`seed_for_attempt` 適用後）
    pub seed: u64,
}

/// JSONL 1 行に対応する耐久レコード（DESIGN §11.3）．
///
/// 1 測定（1 instrument）につき 1 レコード．同一生成に属するレコードは `model` / `response` を
/// 共有する（再開時に Trial を復元するために全レコードへ載せる）．
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub case: CaseId,
    pub attack: AttackRef,
    pub attempt: u32,
    pub seed: u64,
    pub model: ModelRef,
    pub response: Response,
    pub measurement: Measurement,
}

impl CheckpointRecord {
    /// このレコードの完了タプル．
    pub fn tuple_key(&self) -> TupleKey {
        TupleKey {
            case: self.case.clone(),
            attack: self.attack.clone(),
            instrument_name: self.measurement.instrument.name.clone(),
            instrument_version: self.measurement.instrument.version.clone(),
            attempt: self.attempt,
            seed: self.seed,
        }
    }
}

/// append-only JSONL のチェックポイント（DESIGN §11.3）．
pub struct Checkpoint {
    path: PathBuf,
    inner: Mutex<Inner>,
}

struct Inner {
    /// 完了タプル集合（スキップ判定に使う）
    done: HashSet<TupleKey>,
    /// 読み込み済み + 追記済みの全レコード（Trial 復元に使う）
    records: Vec<CheckpointRecord>,
}

impl Checkpoint {
    /// 既存 JSONL を読み込んでチェックポイントを開く（無ければ空で始める）．
    ///
    /// 壊れた最終行（生成途中の kill でちぎれた行）はスキップし，堅牢に読み込む．
    pub async fn load(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let path = path.as_ref().to_path_buf();
        let mut done = HashSet::new();
        let mut records = Vec::new();

        match tokio::fs::read_to_string(&path).await {
            Ok(text) => {
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    // ちぎれた行・不正 JSON はスキップ（append-only の堅牢性）
                    if let Ok(rec) = serde_json::from_str::<CheckpointRecord>(line) {
                        done.insert(rec.tuple_key());
                        records.push(rec);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        Ok(Self {
            path,
            inner: Mutex::new(Inner { done, records }),
        })
    }

    /// 与えたタプル群が**すべて**完了済みか（1 生成をスキップしてよいかの判定）．
    pub async fn contains_all(&self, keys: &[TupleKey]) -> bool {
        let inner = self.inner.lock().await;
        keys.iter().all(|k| inner.done.contains(k))
    }

    /// 単一タプルが完了済みか．
    pub async fn contains(&self, key: &TupleKey) -> bool {
        let inner = self.inner.lock().await;
        inner.done.contains(key)
    }

    /// 完了タプル数．
    pub async fn done_count(&self) -> usize {
        self.inner.lock().await.done.len()
    }

    /// 1 生成分のレコード群を**まとめて**追記する（DESIGN §11.3）．
    ///
    /// - 完了タプルガード: 既に完了済みのタプルは書かない（冪等）．
    /// - バッチ追記: 未完了レコードを 1 度の `write_all` + `flush` + `sync_all`（fsync）で書く．
    ///   生成の原子性を保ち，途中 kill で一部だけ残る事態を避ける．
    pub async fn append_records(
        &self,
        records: Vec<CheckpointRecord>,
    ) -> Result<(), std::io::Error> {
        let mut inner = self.inner.lock().await;

        // ガード: まだ書いていないレコードだけを対象にする
        let mut buf = String::new();
        let mut to_add: Vec<CheckpointRecord> = Vec::new();
        for rec in records {
            let key = rec.tuple_key();
            if inner.done.contains(&key) {
                continue;
            }
            let line = serde_json::to_string(&rec)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            buf.push_str(&line);
            buf.push('\n');
            to_add.push(rec);
        }
        if to_add.is_empty() {
            return Ok(());
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(buf.as_bytes()).await?;
        file.flush().await?;
        // 耐久化（fsync）: 途中 kill でも書けた行は失われない
        file.sync_all().await?;

        for rec in to_add {
            inner.done.insert(rec.tuple_key());
            inner.records.push(rec);
        }
        Ok(())
    }

    /// 読み込み時点までに完了した Trial 群を復元する（DESIGN §5.5）．
    ///
    /// `(case, attack, attempt)` でレコードをグルーピングし，同一生成に属する測定を
    /// `Trial.measurements: Vec<Measurement>` へ束ね直す．出現順を保つ．
    pub async fn resumed_trials(&self) -> Vec<Trial> {
        let inner = self.inner.lock().await;
        Self::group_trials(&inner.records)
    }

    fn group_trials(records: &[CheckpointRecord]) -> Vec<Trial> {
        // 出現順を保ちながら (case, attack, attempt) でまとめる
        let mut index: std::collections::HashMap<(CaseId, AttackRef, u32), usize> =
            std::collections::HashMap::new();
        let mut trials: Vec<Trial> = Vec::new();

        for rec in records {
            let key = (rec.case.clone(), rec.attack.clone(), rec.attempt);
            match index.get(&key) {
                Some(&i) => {
                    trials[i].measurements.push(rec.measurement.clone());
                }
                None => {
                    let i = trials.len();
                    index.insert(key, i);
                    trials.push(Trial {
                        case: rec.case.clone(),
                        attempt: rec.attempt,
                        model: rec.model.clone(),
                        attack: rec.attack.clone(),
                        response: rec.response.clone(),
                        measurements: vec![rec.measurement.clone()],
                    });
                }
            }
        }
        trials
    }

    /// 全レコードを temp へ書き出し → fsync → rename で原子的に置き換える（DESIGN §11.3）．
    ///
    /// append-only で肥大化した JSONL を整理する用途．rename は同一ディレクトリ内で原子的．
    pub async fn compact(&self) -> Result<(), std::io::Error> {
        let inner = self.inner.lock().await;
        let tmp = self.path.with_extension("jsonl.tmp");

        let mut buf = String::new();
        for rec in &inner.records {
            let line = serde_json::to_string(rec)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            buf.push_str(&line);
            buf.push('\n');
        }

        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
                .await?;
            file.write_all(buf.as_bytes()).await?;
            file.flush().await?;
            file.sync_all().await?;
        }
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pil_core::{FinishReason, InstrumentRef, MeasurementParams, SourceRef, Verdict};

    fn src() -> SourceRef {
        SourceRef::new("repo", "deadbeef", "data/x.csv", 0)
    }

    fn case_id(prompt: &str) -> CaseId {
        CaseId::derive(prompt, None, &src())
    }

    fn instrument(name: &str, version: &str) -> InstrumentRef {
        InstrumentRef {
            name: name.into(),
            version: version.into(),
            source: src(),
            params: MeasurementParams {
                response_clip_tokens: None,
                judge_model: None,
                temperature: 0.0,
            },
        }
    }

    fn record(
        prompt: &str,
        attempt: u32,
        seed: u64,
        name: &str,
        version: &str,
    ) -> CheckpointRecord {
        CheckpointRecord {
            case: case_id(prompt),
            attack: AttackRef::identity(),
            attempt,
            seed,
            model: ModelRef::new("mock", "mock-1", None),
            response: Response {
                text: "ok".into(),
                finish_reason: FinishReason::Stop,
                prompt_tokens: Some(1),
                completion_tokens: Some(1),
                reached_clip_limit: false,
            },
            measurement: Measurement {
                verdict: Verdict::Success,
                score: None,
                instrument: instrument(name, version),
                raw: "raw".into(),
            },
        }
    }

    fn tmp_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pil-runner-ckpt-{tag}-{}-{n}-{nanos}.jsonl",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn append_then_reload_recovers_tuples() {
        let path = tmp_path("reload");
        {
            let ckpt = Checkpoint::load(&path).await.unwrap();
            ckpt.append_records(vec![
                record("p1", 1, 1, "refusal", "gcg"),
                record("p1", 1, 1, "rubric", "v1"),
            ])
            .await
            .unwrap();
            assert_eq!(ckpt.done_count().await, 2);
        }
        // 別インスタンスで再読込 → 完了タプルが復元される
        let reloaded = Checkpoint::load(&path).await.unwrap();
        assert_eq!(reloaded.done_count().await, 2);
        assert!(
            reloaded
                .contains(&record("p1", 1, 1, "refusal", "gcg").tuple_key())
                .await
        );
        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn append_is_idempotent_guarded() {
        let path = tmp_path("idem");
        let ckpt = Checkpoint::load(&path).await.unwrap();
        let recs = vec![record("p1", 1, 1, "refusal", "gcg")];
        ckpt.append_records(recs.clone()).await.unwrap();
        // 同一タプルを再追記してもガードで無視される（二重記録しない）
        ckpt.append_records(recs.clone()).await.unwrap();
        ckpt.append_records(recs).await.unwrap();
        assert_eq!(ckpt.done_count().await, 1);
        // ファイル上も 1 行だけ
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(text.lines().filter(|l| !l.trim().is_empty()).count(), 1);
        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn contains_all_gates_generation_skip() {
        let path = tmp_path("all");
        let ckpt = Checkpoint::load(&path).await.unwrap();
        let keys = vec![
            record("p1", 1, 1, "refusal", "gcg").tuple_key(),
            record("p1", 1, 1, "rubric", "v1").tuple_key(),
        ];
        assert!(!ckpt.contains_all(&keys).await); // まだ何も無い
        ckpt.append_records(vec![record("p1", 1, 1, "refusal", "gcg")])
            .await
            .unwrap();
        assert!(!ckpt.contains_all(&keys).await); // 片方だけ → まだスキップ不可
        ckpt.append_records(vec![record("p1", 1, 1, "rubric", "v1")])
            .await
            .unwrap();
        assert!(ckpt.contains_all(&keys).await); // 両方揃った → スキップ可
        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn resumed_trials_group_measurements_per_generation() {
        let path = tmp_path("group");
        let ckpt = Checkpoint::load(&path).await.unwrap();
        // 同一生成 (p1, identity, attempt 1) に 2 測定，別生成 (p1, attempt 2) に 1 測定
        ckpt.append_records(vec![
            record("p1", 1, 1, "refusal", "gcg"),
            record("p1", 1, 1, "rubric", "v1"),
        ])
        .await
        .unwrap();
        ckpt.append_records(vec![record("p1", 2, 2, "refusal", "gcg")])
            .await
            .unwrap();
        let trials = ckpt.resumed_trials().await;
        assert_eq!(trials.len(), 2);
        let a1 = trials.iter().find(|t| t.attempt == 1).unwrap();
        assert_eq!(a1.measurements.len(), 2); // 1 生成に複数測定
        let a2 = trials.iter().find(|t| t.attempt == 2).unwrap();
        assert_eq!(a2.measurements.len(), 1);
        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn compact_rewrites_atomically() {
        let path = tmp_path("compact");
        let ckpt = Checkpoint::load(&path).await.unwrap();
        ckpt.append_records(vec![record("p1", 1, 1, "refusal", "gcg")])
            .await
            .unwrap();
        ckpt.compact().await.unwrap();
        // compact 後も再読込で復元できる
        let reloaded = Checkpoint::load(&path).await.unwrap();
        assert_eq!(reloaded.done_count().await, 1);
        tokio::fs::remove_file(&path).await.ok();
    }

    #[tokio::test]
    async fn load_skips_corrupt_trailing_line() {
        let path = tmp_path("corrupt");
        let ckpt = Checkpoint::load(&path).await.unwrap();
        ckpt.append_records(vec![record("p1", 1, 1, "refusal", "gcg")])
            .await
            .unwrap();
        // 途中 kill でちぎれた行を模して不正 JSON を追記
        {
            let mut f = OpenOptions::new().append(true).open(&path).await.unwrap();
            f.write_all(b"{\"case\": \"broke").await.unwrap();
            f.flush().await.unwrap();
        }
        let reloaded = Checkpoint::load(&path).await.unwrap();
        // 壊れた行はスキップされ，正常な 1 件だけ復元
        assert_eq!(reloaded.done_count().await, 1);
        tokio::fs::remove_file(&path).await.ok();
    }
}
