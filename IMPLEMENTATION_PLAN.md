# prompt-injection-lab — 実装計画書 (Phase 1)

> 本書は [`DESIGN.md`](./DESIGN.md) に基づく実装計画である．§ 参照はすべて `DESIGN.md` の節番号を指す．
> Phase 1（データセット型で縦串を通す，§10）を詳細化し，Phase 2/3 は概要のみ示す．

- **対象フェーズ**: Phase 1
- **作成日**: 2026-07-18
- **前提設計**: `DESIGN.md`（同リポジトリ）

---

## 0. 成功条件（Definition of Done — Phase 1）

Phase 1 の完了は，§10「Phase 1 で実証できるもの」がすべて **CLI から再現可能**であることをもって定義する．

- [x] **測定器の信頼性開示**: `judge-comparison.csv` から §3.1 の数値（HarmBench FPR = 0.268・水増し 1.34x 等）を回帰テストで完全再現．`pil reliability` が CLI から再現
- [x] **単一設定 ASR の否定**: `pil-attacks` の文献既存変換で attack バリアント跨ぎの **union coverage** を出す（`pil report` で単一最良 72.2% < union 100.0% を確認）
- [x] **多試行 ASR と信頼区間**: Anthropic 式 1/10/100 回開示を Wilson / Case 単位 bootstrap 区間つきで出す（`asr@k` 曲線）
- [x] **ベンチマーク間の非独立性**: `ContentKey`（§5.2）で JBB∩AdvBench=11 / JBB∩HarmBench=9 / AdvBench∩HarmBench=0（§3.4）を実データから自動検出
- [x] **過剰拒否**: JBB benign 100 件で over-refusal を測る（`pil report`）
- [x] 三値判定（`Undecidable`）が全経路で保持され，二値化は件数併記つきの明示操作でのみ行われる（§5.3）
- [x] 既定ビルドが **ネットワーク非依存**でテストが通る（`cargo tree -i reqwest` = 該当なし，175 tests network-free）（§6.1）

---

## 1. 全体像とマイルストーン依存

```
M0 workspace 基盤
  └─ M1 pil-core (型定義)  ……… 全ての土台
        ├─ M2 pil-metrics::reliability  ← 最初の差別化点・network-free（§10 着手順）
        ├─ M3 pil-metrics::instrument   （parse/非LLM判定は先行，LLM判定は M4 後）
        ├─ M4 pil-llm                    （Ollama 既定・feature gate・cache）
        ├─ M5 pil-bench-*  (loaders)     （submodule 直読・native）
        └─ M6 pil-attacks  (transforms)
              └─ M7 pil-runner           （多試行・並行・レート制御・中断再開）← M4,M6
                    └─ M8 pil-metrics::aggregate  （Wilson/bootstrap/union/EnvKind強制）
                          └─ M9 pil-report        （二値化+undecidable併記・ContentKey重複）
                                └─ M10 pil-cli
                                      └─ M11 End-to-end 検証
```

**クリティカルパス**: M0 → M1 → M2 →（M3/M4/M5/M6 は並行可）→ M7 → M8 → M9 → M10 → M11．

M2 を M1 直後に置くのは §10 の指針どおり — 正解データが手元にありネットワーク不要で，かつ `pil-core` の型が実データに耐えるかを同時に検証できるため．

---

## 2. マイルストーン詳細

各マイルストーンは «成果物 / 主なタスク / DoD / 設計参照» で記す．チェックボックスは進捗に応じて更新する（`[ ]`未着手 / `[→]`進行中 / `[x]`完了）．

### M0 — workspace 基盤

- **成果物**: ビルドの通る空の Cargo workspace，固定済み submodule，CI 雛形
- タスク
  - [x] `Cargo.toml`（workspace）+ §9 の全 crate ディレクトリを空 lib/bin で作成
  - [x] `third_party/` に §7.1 の submodule を **フル SHA 固定**で追加（AdvBench / HarmBench / StrongREJECT(alexandrasouly) / JBB-Behaviors / **dsbowen/strong_reject**）
  - [x] `dsbowen/strong_reject` の **ライセンス・最終コミット日を確認**し §7.1 表を確定（MIT (c) 2024 Dillon Bowen / 2025-07-07）
  - [x] cargo feature 骨組み: 既定は network-free．`ollama` / `openai` / `anthropic` / `gemini` を gate（§6.1）
  - [x] CI: `cargo test`（既定 feature）・`cargo clippy -D warnings`・`cargo fmt --check`
  - [x] `rust-toolchain.toml` でツールチェイン固定
- **DoD**: ✅ `cargo build` / `cargo test` が緑．submodule が SHA で固定され `git submodule status` が一致

### M1 — `pil-core`（型定義）

- **成果物**: §5 の全型と決定論的導出，serde ラウンドトリップ
- タスク
  - [x] `SourceRef`（§5.1）
  - [x] `Case` + `EnvKind`（§5.2）
  - [x] `CaseId = blake3(canonical(prompt) ‖ canonical(context) ‖ SourceRef)`，表示は先頭16 hex（§5.2）
  - [x] `ContentKey = blake3(normalize(prompt) ‖ normalize(context))` と `normalize`（小文字化・空白正規化・末尾ピリオド除去，§3.5）
  - [x] `Verdict` / `UndecidableReason`（§5.3）
  - [x] `InstrumentRef` / `MeasurementParams` / `Measurement`（§5.4）
  - [x] `Trial`（§5.5）
  - [x] `AttackRef` / `Transform`（§5.6）
  - [x] `ModelRef`，および **`Response` 型を新規定義**（`text` / `finish_reason` / `token 数` / クリップ到達フラグ — `ResponseTruncated` 判定の根拠，§11.4 残タスク）
- **DoD**: ✅ 全型 serde JSON ラウンドトリップ test 緑（17 tests）．`CaseId` は「同一テキスト・異なる source → 別 id」を，`ContentKey` は「同一テキスト・異なる source → 同一 key」を test で保証（§3.3 / §3.4）

### M2 — `pil-metrics::reliability`（差別化点・network-free）

- **成果物**: judge 信頼性指標と §3.1 の回帰テスト
- タスク
  - [x] `data/judge-comparison.csv`（JBB 300 件）を読む薄いローダ（`pil-bench-jbb` の一部 or 専用フィクスチャ，M5 で本体に統合）
  - [x] 指標算出: recall / FPR / precision / F1 / accuracy（vs 人手多数決）
  - [x] 報告 ASR と真 ASR の乖離（水増し倍率）
  - [x] アノテータ間 Cohen's kappa（測定精度の上限）
  - [x] 判定器間の一致率
- **DoD**: ✅ §3.1 の値を**完全一致で再現**する回帰テスト（`harmbench_cf`: recall 0.873 / FPR 0.268 / …，真 ASR 36.7%，kappa 0.809/0.826/0.886，`harmbench_cf` の一致率 77〜78%）．ネットワーク・LLM を一切呼ばない（§8.4）— 4 tests 緑
- **参照**: §8.4 / §3.1．依存の向きは `reliability → instrument`（§8.1）

### M3 — `pil-metrics::instrument`（4種の測定器）

- **成果物**: `Instrument` trait と4実装．parse/scoring は canned 出力で network-free test
- タスク
  - [x] `trait Instrument { reference(); measure(case, response) -> Measurement }`（§8.2）．適用不可は `Undecidable{NotApplicable}`
  - [x] **文字列マッチ**（AdvBench refusal，`target` 列使用）．`target=None` の Case は `NotApplicable`（§3.2 / §5.2）
  - [x] **StrongREJECT ルーブリック v1/v2**: プロンプト文言を native 実装（v1=alexandrasouly，v2=dsbowen，§3.7 / §7.1）．採点式 v1 `((c+s)/2−1)/4`・v2 `(1−refusal)(c+s−2)/8`，パース失敗は `Undecidable{ParseFailure}`（§3.6）
  - [x] **HarmBench cls**: テンプレート選択ロジック（`context`/`multimodal`/else，§8.2），yes/no 以外は `Undecidable{ClassifierOutOfVocab}`
  - [x] **MinHash 照合**（HarmBench copyright 100 件）: Jaccard 閾値 0.6，window/overlap（book 300/200・lyrics 50/40），参照ハッシュ `data/copyright_classifier_hashes/{id}.pkl`，`hash_check` タグで分岐（§8.2）
- **DoD**: ✅ 文字列マッチ・MinHash・ルーブリック採点/パースは **canned な judge 生出力フィクスチャ**で network-free test 緑（46 tests，`Judge` 抽象で LLM 生成経路を注入可能に）．三値 Undecidable 全経路（NotApplicable/ParseFailure/ClassifierOutOfVocab）を被覆．`.pkl` は `scripts/convert_copyright_hashes.py` で JSON 中間形式へ前処理
- **注**: `.pkl` 読取は Python pickle 依存を避け，ハッシュ値を事前抽出した中間形式（JSON/バイナリ）へ変換する前処理を native 側に持つ

### M4 — `pil-llm`（プロバイダ抽象）

- **成果物**: Ollama 既定のプロバイダ層，cache，logprobs 公開
- タスク
  - [x] `LlmConfig`（temperature/seed/max_tokens/system）・`CallMetadata`（model/endpoint/temp/seed/cache_hit）（§6.1）
  - [x] Ollama バックエンド（既定）．**起動時に `>= 0.12.11` を検査**し満たさなければ明示失敗（§6.3 / §11.1）
  - [x] `top_logprobs` を公開する API（§6.3）．OpenAI 互換経路での実返却は Phase 2 で実測（native `/api/generate` は確定）
  - [x] OpenAI / Anthropic / Gemini バックエンドを feature gate（Phase 1 は必須でない骨組み）
  - [x] `CachingClient`: キー `hash(rendered_prompt + model + params + attempt + seed)`．`rendered_prompt` は変換適用後の最終送信文（§6.2）．監査用に `(CaseId, AttackRef)` を併記記録
  - [x] 既定ビルド（network-free）でのモック/キャッシュ再生経路
- **DoD**: ✅ モック応答でキャッシュ命中/分離の test 緑（同一 Case の異なる変換が別キー，異なる attempt が別キー — §6.2）．既定ビルドは reqwest 非依存を確認．`ollama` feature ビルド緑．seed=base.wrapping_add(attempt)．12 tests

### M5 — `pil-bench-*`（ローダ，native 直読）

- **成果物**: 4ベンチの native ローダ．submodule 実ファイルを `(path,row)` 直読し `Vec<Case>` を返す（§7.3）
- タスク
  - [x] `pil-bench-advbench`: 原典 `goal,target`（520，target あり，§3.2）
  - [x] `pil-bench-harmbench`: **ファイル別可変スキーマ**（text 6列/multimodal 9列 列順違い/extra 5列/`2_behaviors` 6列），`ContextString` は RFC4180 複数行 quoted，`Tags` は `", "` 区切り（§9.1）
  - [x] `pil-bench-strongreject`: full 313 + small 60（順序が違う・部分列でない），rubric v1/v2 プロンプトの `SourceRef`（v1→alexandrasouly，v2→dsbowen）（§9.1 / §7.1）
  - [x] `pil-bench-jbb`: harmful 100 + benign 100（`benign=true`）+ judge-comparison 300．M2 の薄いローダをここへ統合
  - [x] 各ローダは **ネットワーク取得をしない**．commit は submodule 固定 SHA を `SourceRef.commit` に転記（§7.3）
- **DoD**: ✅ 各ベンチの件数が設計どおり（AdvBench 520 / HarmBench text_all 400・multimodal 110 / SR 313+60 / JBB 100+100+300）．CSV パーサ回帰テスト（§9.1 の落とし穴：埋め込み改行・引用符・列順）緑．`Case.source` に正しい `(upstream,commit,path,row)`．21 tests．Tags census: context 100 / hash_check 100 / book 50 / lyrics 50

### M6 — `pil-attacks`（変換・union のバリアント軸）

- **成果物**: `Transform` 実装群と決定論的レンダラ
- タスク
  - [x] `Identity` / `Base64` / `Leetspeak` / `Translate{lang}` / `Roleplay{template_id}` / `RefusalSuppression`（§5.6）
  - [x] `render(case, attack) -> rendered_prompt`．全変換**決定論的**（roleplay はテンプレ ID 固定，乱数不使用）
  - [x] 各 `AttackRef.source` に再現元（論文/実装）を記録（§1.4 の「既存手法の再現に限る」を型で担保）
  - [x] `Translate` は Phase 1 の既定言語セットを固定（オフライン辞書 or 固定翻訳表；外部翻訳 API に依存しない）
- **DoD**: ✅ 各変換のゴールデン test（入力→出力が固定）．`render` が同一入力で必ず同一出力（14 tests）．`Translate` はオフライン固定（zu/gd/hmn/gn）．`render` は `attack.transform` のみに依存し source を参照しない（cache key を汚さない）

### M7 — `pil-runner`（多試行・並行・レート制御・中断再開）

- **成果物**: Case×Attack×attempt を回す実行器
- タスク
  - [x] 多試行ループ（attempt 1..=N）．`seed = f(attempt)` の規約を確定（再現性と独立サンプルの両立，§11.4）
  - [x] 有界並行（`tokio::sync::Semaphore` 等）
  - [x] レート制御: プロバイダ毎 token-bucket（RPM/TPM），429 は `Retry-After` 尊重の指数バックオフ + ジッタ（§11.3）
  - [x] 中断再開: `(CaseId, instrument, attempt, seed)` 単位の append-only JSONL．再起動時に完了タプルをスキップ．追記は atomic write/rename で冪等（§11.3）
  - [x] 1生成に複数測定器をぶら下げる（`Trial.measurements: Vec<Measurement>`，§5.5）
- **DoD**: ✅ 途中 kill → 再開で二重生成が起きない test（冪等性，resume で完了タプル再生成 0）．レート制御のバックオフ + token-bucket が単体 test で検証される（24 tests）

### M8 — `pil-metrics::aggregate`（集計・EnvKind 強制）

- **成果物**: ASR・信頼区間・union・多試行の集計と比較可能性の強制
- タスク
  - [x] 単発 ASR + **Wilson score 区間**（既定）．Clopper–Pearson はオプション（§11.3）
  - [x] 多試行 ASR 曲線 / union coverage / judge 間差分 → **Case 単位 bootstrap**（percentile/BCa，seeded）（§11.3）
  - [x] union coverage `coverage(case)=1 iff ∃v∈V. Success`，behavior 群で `mean_case`（§5.6）
  - [x] **EnvKind 比較可能性の強制**: 入力を `by_env: BTreeMap<EnvKind, Vec<Trial>>`，結果は必ず EnvKind タグ付き．横断集計は `unsafe_cross_env` 明示フラグ + 警告刻印．`InstrumentRef` 跨ぎの単純平均も既定禁止（§8.1）
  - [x] `Undecidable` は分母から除外し件数を保持（§5.3）
- **DoD**: ✅ Wilson/CP/bootstrap(percentile+BCa) の数値がテストベクタと一致（50/100→[0.4038,0.5962] 等）．EnvKind/InstrumentRef 跨ぎ集計は `CrossEnv`/`CrossInstrument` マーカー(private field)無しでは**出せない**ことを test で保証（pil-metrics 60 tests）

### M9 — `pil-report`（提示・二値化・重複検出）

- **成果物**: 人間可読レポートと機械可読出力
- タスク
  - [x] 二値化は明示操作，潰した undecidable 件数を必ず併記（§5.3）
  - [x] 信頼区間と undecidable 件数を常に併記（§8.1）
  - [x] `ContentKey` によるベンチ横断重複検出 → §3.4 の非独立性を自動レポート（§3.4 / §5.2）
  - [x] `reliability` 出力の整形（§8.4 の指標）
- **DoD**: ✅ §3.4 の重複（JBB∩AdvBench=11 / JBB∩HarmBench=9 / AdvBench∩HarmBench=0）を実データから ContentKey で完全再現・報告（正規化でも乖離なし，16 tests）

### M10 — `pil-cli`

- **成果物**: サブコマンドと suite 定義
- タスク
  - [x] `suites/*.toml`（実験セット定義）読取
  - [x] サブコマンド: `reliability`（M2）/ `run`（生成+測定）/ `report`（集計+提示）
  - [x] 結果を `results/{subcommand}_YYYYMMDD_HHMMSS/` に保存（タイムスタンプ・provenance 同梱）
- **DoD**: ✅ 3サブコマンド（reliability/run/report）が end-to-end で走り，成果物（JSONL/JSON/provenance.json）が `results/{sub}_TS/` に落ちる（11 tests，workspace 計 175 緑）

### M11 — End-to-end 検証（Phase 1 総合）

- タスク
  - [x] §0 の DoD 全項目を CLI 実行で再現
  - [x] `reliability` が §3.1 を再現
  - [x] 4ベンチ static 実行で union coverage・多試行 ASR・過剰拒否・非独立性を出力
  - [x] Phase 1 で**実証できないもの**（環境種別跨ぎ，§10）が「跨げない」ことを明示的に報告（`pil report` が StaticPrompt 単一を明示）
- **DoD**: ✅ §0 のチェックリストが全て緑（CLI 実行で再現確認済み）．README 反映の材料が揃う

---

## 3. テスト戦略

- **ネットワーク非依存を既定に**（§6.1）: 既定 feature ビルドで `cargo test` が LLM/HTTP を一切呼ばない
- **回帰フィクスチャ**:
  - `judge-comparison.csv`（300）→ §3.1 の数値を期待値に（M2）
  - canned な judge 生出力 → ルーブリック parse/scoring（M3）
  - submodule 実ファイル → ローダ件数・スキーマ（M5）
- **決定論の担保**: `CaseId`/`ContentKey`/`render`/bootstrap すべてゴールデン or seeded
- **冪等性**: runner の中断再開（M7）
- **プロパティテスト**: CSV パーサ（埋め込み改行・引用符），正規化関数
- **統計の正しさ**: Wilson / bootstrap をテストベクタで検証（M8）

---

## 4. 技術スタック（推奨・確定は実装時）

| 用途 | 候補 | 備考 |
|---|---|---|
| シリアライズ | `serde` / `serde_json` | 全型 derive |
| CSV | `csv` | RFC4180 quoting 既定（§9.1）|
| ハッシュ | `blake3` | `CaseId` / `ContentKey`（§5.2）|
| MinHash | 自作（小規模）| Jaccard 閾値 0.6（§8.2）|
| 統計 | `statrs` + 自作 bootstrap | Wilson は正規分位点，bootstrap は seeded |
| 乱数 | `rand` + `rand_chacha` | seeded・決定論 |
| 非同期/並行 | `tokio` | runner の並行 + semaphore（§11.3）|
| レート制御 | `governor` or 自作 token-bucket | RPM/TPM（§11.3）|
| HTTP | `reqwest`（feature gate）| 既定ビルドから除外（§6.1）|
| CLI | `clap` | サブコマンド |
| 設定 | `toml` | `suites/*.toml` |
| エラー | `thiserror`（lib）/ `anyhow`（bin）| 判定不能は型で表現（§5.3）|

---

## 5. リスクと残タスク

- **`dsbowen/strong_reject` の pin 情報**（ライセンス・最終コミット日）を M0 で確定（§7.1 / §11.4）
- **Ollama OpenAI 互換の logprobs 実返却**は Phase 2 で実測（native は確定，§6.3）
- **`.pkl` 参照ハッシュ**（HarmBench copyright）: Python pickle を避け中間形式へ前処理（M3）
- **`seed = f(attempt)` 規約**: 多試行の独立性と再現性の両立（M7 / §11.4）
- **`Translate` の再現性**: 外部翻訳 API を使わずオフライン固定（M6）
- **`Response` 型**: `ResponseTruncated` の根拠となる finish_reason/クリップ情報を保持（M1 / §11.4）

---

## 6. Phase 2 / Phase 3（概要）

### Phase 2 — 環境型と横断比較（§10）
- §4.1 の OpenAI 互換シムを実装（`pil-llm` を単一の通り道に）．`base_url` 差し替えは検証済み（§11.1）
- AgentDojo を sidecar で取り込む（native-first：グルーは Rust，環境本体のみ Python，§4.1）
- fine-tuned judge（v1 テンプレート・logit 期待値式）を追加（GGUF 化が前提，§8.3）
- **環境種別跨ぎの比較**がここで初めて成立（§8.1 の EnvKind 強制が効く）

### Phase 3 — 候補（§10）
- 適応型ベンチマークの標準形
- マルチエージェント創発リスク
- AdvBench 近傍重複の定量化（意味的冗長性を埋め込みで測る）

---
*This file was generated by Claude Code.*
