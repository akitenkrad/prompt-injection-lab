[English](architecture.md) | **日本語**

# アーキテクチャ

workspace は `pil-*` crate の集合である（`pil-` = **p**rompt-**i**njection-**l**ab）．現在のツリーは 14 crate からなり，Phase 2 の crate を下表で示す．

| crate | 役割 | Phase 2 |
|---|---|---|
| `pil-core` | 中核型定義．`SourceRef`（同一性の単位 `(repo, commit, path, row)`），`Case`（`target: Option<String>` で AdvBench の二重梱包を型で表現），`EnvKind`，三値 `Verdict`，`InstrumentRef` + `MeasurementParams`，`Measurement`，`Trial`（1 生成に測定器を複数ぶら下げる `measurements: Vec<Measurement>`），および `AttackRef` + `Transform`． | |
| `pil-llm` | プロバイダ抽象（独立実装）．`LlmConfig`，`CallMetadata`（何と話したかを全呼び出しで記録），`hash(rendered_prompt + model + params + attempt + seed)` を鍵とするキャッシュ（多試行が 1 件に潰れない），`top_logprobs` 公開．Ollama 既定 + OpenAI / Anthropic / Gemini を feature gate．OpenAI 互換バックエンドは **tool-calling とモデル名上書き（`with_model_override`）つきで実装済み**で，固定のモデル enum しか受け付けない上流ベンチもローカルモデルへ routing できる． | ● |
| `pil-metrics` | 内部を 3 分割．`instrument`（1 件ずつ判定：文字列マッチ / LLM 生成判定 / LLM logprob 判定 / ハッシュ照合，加えて **fine-tuned StrongREJECT judge** `strongreject-finetuned` — logit 期待値式の測定器），`aggregate`（ASR・信頼区間・union coverage・多試行 ASR，`EnvKind` / `InstrumentRef` で比較可能性を強制，および **StrongREJECT スコア一致度** — 3 つの StrongREJECT judge の連続スコア上の Kendall の一致係数 W），`reliability`（判定器自身の再現率・FPR を測定，第一級）． | ● |
| `pil-bench-advbench` | AdvBench ローダ（原典 `llm-attacks`，`goal,target` スキーマ，520 件）． | |
| `pil-bench-harmbench` | HarmBench ローダ（ファイルごとに異なる 5〜9 列可変スキーマ + Tags の `", "` 分解 + MinHash Jaccard copyright 判定 + cls テンプレート選択）． | |
| `pil-bench-strongreject` | StrongREJECT ローダ + ルーブリック **v1 / v2 を両方 native Rust 再実装**（同一応答への判定差を測る）． | |
| `pil-bench-jbb` | JBB ローダ（harmful 100 + benign 100 + `judge-comparison.csv` 300）． | |
| `pil-bench-agentdojo` | AgentDojo の native-first アダプタ．AgentDojo v0.1.35 を対象に，ケース型・provenance（`EnvKind::Emulated`）・結果 JSON パース・列挙 ingest を担う．環境・ツール・決定論的 scoring は Python に残し，「測定値を変えない」層のみを Rust に持つ．**シム経由でローカルのツール対応モデルに対しライブ実行でき，実装済み**である（single ケースと `--limit N` の batch を持ち，batch は `pil report` がそのまま読める `EnvKind::Emulated` の run dir を出す）． | ● |
| `pil-attacks` | `AttackRef` 変換レンダラ（Identity / Base64 / Leetspeak / Translate / Roleplay / RefusalSuppression）．**公開手法の再現のみ**で新規攻撃は作らず，`source: Option<SourceRef>` で再現元 provenance を刻む． | |
| `pil-runner` | 多試行・有界並行（semaphore）・token-bucket レート制御（429 は `Retry-After` 尊重 + 指数バックオフ）・中断再開（`(CaseId, instrument, attempt, seed)` 単位の append-only JSONL，再起動時に完了タプルをスキップ，atomic write/rename で冪等）． | |
| `pil-report` | run 成果物からの集計：undecidable 件数を併記した信頼区間・union coverage・多試行 ASR・過剰拒否・`ContentKey` による非独立性の自動検出．**`EnvKind` ごとの ASR を並置**し，明示的な `--cross-env` 指定時のみ横断プールスカラを警告および **Kendall の一致係数 W** とともに出す．複数の `--run` ディレクトリを受け取り union する． | ● |
| `pil-shim` | OpenAI 互換ローカルシム（制御の反転）．OpenAI 互換エンドポイントを立て，外部 Python ベンチの `base_url` をそこへ向けさせる．OpenAI ⇄ `pil-llm` の純変換は feature 非依存で単体テスト可能とし，HTTP サーバ（axum/tokio）は `shim` feature の裏に置く．エージェント型ベンチに必要な **tool-calling passthrough**（`tools` / `tool_calls` / `developer` ロール）を持つ． | ● |
| `pil-sidecar` | Python sidecar 起動基盤（native-first）．「測定値を変えないグルー」— プロセス起動・環境変数注入・入出力正規化・provenance —を Rust に集約し，Python 側は薄い殻に留める．シムの `base_url` を `OPENAI_BASE_URL`（とダミーの `OPENAI_API_KEY`）に注入し，Python の OpenAI 互換クライアントを単一の `pil-llm` 経路へ routing する．実際のプロセス起動は `sidecar` feature の裏に置く． | ● |
| `pil-cli` | `pil` バイナリ（`reliability` / `run` / `report`，加えて `agentdojo-live` feature の裏の `agentdojo`，`strongreject-judge` feature の裏の `strongreject-judge`，`strongreject-concordance` feature の裏の `strongreject-concordance` — rubric v1 / v2 + fine-tuned judge のライブ 3 判定器 StrongREJECT 一致度）． | ● |

上流ローダは native Rust で持ち，上流の Python ローダ（`main` ブランチや個人アカウントの raw URL をハードコード取得する）は一切経由しない．これにより SHA 固定が実効化され，「どの SHA の何行目か」が全 `Case` に保証される．データの読み取り・正規化・`SourceRef` 付与はグルーであり，Rust に置いても測定値は変わらない．

**fine-tuned StrongREJECT judge** も同じ native-first の分割に従う：モデル固有の logit 抽出（base `google/gemma-2b` + LoRA アダプタ `qylu4156/strongreject-15k-v1`）を小さな transformers+peft の Python sidecar（`crates/pil-metrics/python/score_dist.py`）に置き，採点式（採点トークン上の softmax · linspace 期待値）・三値判定・provenance は Rust 側に留める．Python 側は採点トークンの生 logits を返すだけで，測定値を動かしうる計算は一切行わない．

## 制御の反転（実装済み）

環境型ベンチ（AgentDojo 等）を sidecar で素直に取り込むと「Rust が Python を呼び，Python が自分でモデルを呼ぶ」形になる．これでは**モデル呼び出しが 2 系統に分裂**し，温度・シード・リトライ・メタデータ記録が揃わなくなる — 比較可能性の破綻がリポジトリ内部で再発する．

そこで制御を反転させており，これは実装済みである：**Rust の `pil-shim` プロセスが OpenAI 互換のローカルエンドポイント（`/v1/chat/completions`）を立て，Python 側ベンチの client の `base_url` をそこへ向ける**．sidecar（`pil-sidecar`）はベンチの irreducible な本体（環境とツール実行）だけを走らせ，モデル呼び出しは全て Rust に戻り，単一の `pil-llm` 経路に集約される（温度・seed・cache・metadata・rate-limit が全経路で揃う）．シムは tool-calling passthrough を持つのでエージェント型ベンチがそのまま動き，**モデル名上書きはシム境界に置かれる**ため，固定のモデル enum しか受け付けない上流もローカルモデルへ routing される．Ollama 自体が OpenAI 互換であるため，この経路は既に踏み固められている．

**native-first** — Python sidecar には irreducible な部分（環境・ツール本体で，Rust に書き直すと「同じベンチマーク」ではなく「我々の再実装」になってしまう箇所）だけを残す．それ以外 — プロセス配線・IPC・OpenAI 互換シム・tool-calling スキーマ変換・入出力正規化・シリアライズ・キャッシュ・レート制限・メタデータ・エラー／リトライ制御 — は全て native Rust に集約する．Python 側に書くと制御が再び分裂するためである．判断基準は「Rust に移して測定値が変わりうるか」— 変わるなら Python 温存，変わらないなら Rust．

## `EnvKind` 跨ぎの報告

第 2 の環境種別が存在する以上，集計はそれを黙って平均してはならない．`pil-report`（`pil-metrics` と協働）は **`EnvKind` ごとの ASR を並置**し，既定では横断スカラを一切出さない．明示的な `--cross-env` opt-in を指定したときのみ，プール値を警告および **Kendall の一致係数 W** とともに開示する．W は結論が judge／環境にどれだけ依存するかを測る尺度である．共通の測定器・ケースを持たない環境間では W を未定義として正しく報告し，比較不能性を平均に隠さず具体化する．`pil report` は複数の `--run` ディレクトリも受け取り union するので，静的プロンプトの run と emulated の run を一緒に集計しつつ `EnvKind` ごとに報告できる．

## 比較可能性メタデータとしての `EnvKind`

`EnvKind`（`StaticPrompt` / `Emulated` / `RealExecutable`）は第一級メタデータであり，2 つのスコアが比較可能かどうかを決める科学的性質である．adapter 種別（`native` / `sidecar`）とは別物で，後者は実装都合にすぎない．`pil-bench-agentdojo` のケースは `EnvKind::Emulated` を帯び，集計 API は同一 `EnvKind` 内でのみスコアを比較する．
