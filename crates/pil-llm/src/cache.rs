//! `CachingClient` とキャッシュキー（DESIGN §6.2 / §11.3）．
//!
//! **`socsim-llm` の `hash(prompt + model)` をそのまま流用すると壊れる箇所**（§6.2）．
//! 多試行 ASR（同一プロンプトの独立試行）を潰さないため，キーに `attempt` と `seed` を含める．
//! さらに `rendered_prompt` を含めることで，同一 Case の異なる変換（union coverage の各
//! バリアント，§5.6）が別キーに分かれる．監査のため `(CaseId, AttackRef)` を entry に併記する．

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use pil_core::{AttackRef, CaseId, ModelRef, Response};

use crate::config::{seed_for_attempt, CallMetadata};
use crate::error::LlmError;
use crate::provider::{GenerateOutput, GenerateRequest, LlmProvider, TokenLogprobs, ToolCall};

/// キャッシュキーのドメインタグ（他ハッシュ空間との分離）．
const CACHE_KEY_DOMAIN: &str = "pil.llm.cache.v1";

/// «長さ（u64 LE）‖ 本体» でフレーミングして更新する（連結の曖昧さを避ける）．
fn frame(h: &mut blake3::Hasher, bytes: &[u8]) {
    h.update(&(bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

/// `Option<&str>` を «presence フラグ ‖ 本体» で更新（None と Some("") を区別）．
fn frame_opt_str(h: &mut blake3::Hasher, value: Option<&str>) {
    match value {
        None => frame(h, b"\x00"),
        Some(s) => {
            frame(h, b"\x01");
            frame(h, s.as_bytes());
        }
    }
}

/// `Option<u32>` を «presence フラグ ‖ LE バイト» で更新．
fn frame_opt_u32(h: &mut blake3::Hasher, value: Option<u32>) {
    match value {
        None => frame(h, b"\x00"),
        Some(n) => {
            frame(h, b"\x01");
            frame(h, &n.to_le_bytes());
        }
    }
}

/// キャッシュキー = `blake3(rendered_prompt + model + params + attempt + seed)`（DESIGN §6.2）．
///
/// - `rendered_prompt`（= `req.prompt`）を含めるため，**同一 Case の異なる変換は別キー**．
/// - `attempt` と `seed`（= `seed_for_attempt(config.seed, attempt)`）を含めるため，
///   **異なる attempt は別キー**（多試行が 1 件に潰れない）．
///
/// `params` は温度・max_tokens・system・top_logprobs 要求を含む（ASR を動かす一方で
/// 報告されがちなパラメタ，§3.9）．
pub fn cache_key(req: &GenerateRequest) -> String {
    let seed = seed_for_attempt(req.config.seed, req.attempt);
    let mut h = blake3::Hasher::new();
    frame(&mut h, CACHE_KEY_DOMAIN.as_bytes());
    // rendered_prompt
    frame(&mut h, req.prompt.as_bytes());
    // model
    frame(&mut h, req.model.provider.as_bytes());
    frame(&mut h, req.model.model.as_bytes());
    frame_opt_str(&mut h, req.model.endpoint.as_deref());
    // params
    frame(&mut h, &req.config.temperature.to_bits().to_le_bytes());
    frame_opt_u32(&mut h, req.config.max_tokens);
    frame_opt_str(&mut h, req.config.system.as_deref());
    frame_opt_u32(&mut h, req.top_logprobs);
    // tools / tool_choice（§4.1 / M2'）:
    // `tools == None` のときは **一切ハッシュに触れない**ので，M2' 以前のキーとバイト一致を保つ
    // （既存 tests/cache.rs のキー等価/非等価アサーションを壊さない）．
    // ツールが付いた要求だけドメイン分離タグ＋安定シリアライズを畳み込み，別キーに分ける．
    if let Some(tools) = req.tools.as_ref() {
        frame(&mut h, b"tools");
        // ToolSpec は決定論的にシリアライズできる（parameters は既に serde_json::Value）．
        let serialized = serde_json::to_vec(tools).expect("ToolSpec is serializable");
        frame(&mut h, &serialized);
        frame_opt_str(&mut h, req.tool_choice.as_deref());
    }
    // attempt + seed
    frame(&mut h, &req.attempt.to_le_bytes());
    frame(&mut h, &seed.to_le_bytes());
    h.finalize().to_hex().to_string()
}

/// キャッシュ entry に併記する監査情報（DESIGN §6.2）．
///
/// キー材料そのものには入らない `(CaseId, AttackRef)` を残し，どのキーがどの Case の
/// どの攻撃バリアントに対応するかを事後追跡できるようにする．
#[derive(Debug, Clone)]
pub struct CacheAudit {
    pub case_id: CaseId,
    pub attack: AttackRef,
}

impl CacheAudit {
    pub fn new(case_id: CaseId, attack: AttackRef) -> Self {
        Self { case_id, attack }
    }
}

/// 永続化可能なキャッシュ entry（DESIGN §6.2 / §11.3）．
///
/// M7 で `(CaseId, instrument, attempt, seed)` 単位の append-only JSONL として耐久記録に
/// 兼用できるよう，serde でシリアライズ可能に設計する（M4 では in-memory 保持）．
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// キャッシュキー（hex）
    pub key: String,
    /// 監査: 由来 Case の完全 `CaseId`
    pub case_id: String,
    /// 監査: 攻撃バリアント
    pub attack: AttackRef,
    /// 多試行の試行番号
    pub attempt: u32,
    /// 実送信 seed（`seed_for_attempt` 適用後）
    pub seed: u64,
    /// 呼び出したモデル
    pub model: ModelRef,
    /// 生成応答
    pub response: Response,
    /// logprobs（要求時のみ）
    pub logprobs: Option<Vec<TokenLogprobs>>,
    /// モデルが要求したツール呼び出し（無ければ空．§4.1 / M2'）
    ///
    /// 既存 JSONL との後方互換のため，欠損時は空 `Vec`（`default`），空なら書き出さない．
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// 生成時のメタデータ（`cache_hit == false` で保存）
    pub metadata: CallMetadata,
}

/// プロバイダをラップし，プロンプト → 応答をキャッシュするクライアント（DESIGN §6.1 / §6.2）．
///
/// `P` は具体プロバイダでも `Box<dyn LlmProvider>` / `Arc<dyn LlmProvider>` でもよい
/// （`provider` の blanket 実装により）．
pub struct CachingClient<P> {
    inner: P,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl<P: LlmProvider> CachingClient<P> {
    pub fn new(inner: P) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// ラップしている内側プロバイダへの参照．
    pub fn inner(&self) -> &P {
        &self.inner
    }

    /// キャッシュを引きつつ生成する（DESIGN §6.2）．
    ///
    /// キー一致なら内側プロバイダを呼ばずに応答を返し，`metadata.cache_hit = true` を立てる．
    /// ミス時のみ内側を呼び，`(CaseId, AttackRef)` を併記した entry を保存する．
    pub async fn generate_cached(
        &self,
        req: &GenerateRequest,
        audit: &CacheAudit,
    ) -> Result<GenerateOutput, LlmError> {
        let key = cache_key(req);

        // ヒット判定（await をまたいでロックを保持しない）
        {
            let cache = self.cache.lock().expect("cache mutex poisoned");
            if let Some(entry) = cache.get(&key) {
                let mut metadata = entry.metadata.clone();
                metadata.cache_hit = true;
                return Ok(GenerateOutput {
                    response: entry.response.clone(),
                    metadata,
                    logprobs: entry.logprobs.clone(),
                    tool_calls: entry.tool_calls.clone(),
                });
            }
        }

        // ミス: 内側プロバイダを呼ぶ
        let output = self.inner.generate(req).await?;

        let entry = CacheEntry {
            key: key.clone(),
            case_id: audit.case_id.full().to_string(),
            attack: audit.attack.clone(),
            attempt: req.attempt,
            seed: req.effective_seed(),
            model: req.model.clone(),
            response: output.response.clone(),
            logprobs: output.logprobs.clone(),
            tool_calls: output.tool_calls.clone(),
            metadata: output.metadata.clone(),
        };
        self.cache
            .lock()
            .expect("cache mutex poisoned")
            .insert(key, entry);

        Ok(output)
    }

    /// 保持している entry 数．
    pub fn len(&self) -> usize {
        self.cache.lock().expect("cache mutex poisoned").len()
    }

    /// entry が無いか．
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 指定キーの entry を持つか．
    pub fn contains_key(&self, key: &str) -> bool {
        self.cache
            .lock()
            .expect("cache mutex poisoned")
            .contains_key(key)
    }

    /// 全 entry の複製（M7 の JSONL 書き出し等に使う想定）．
    pub fn entries(&self) -> Vec<CacheEntry> {
        self.cache
            .lock()
            .expect("cache mutex poisoned")
            .values()
            .cloned()
            .collect()
    }
}
