# prompt-injection-lab

**横断比較の測定基盤** — 既存のプロンプトインジェクション・ジェイルブレイクベンチマークを，任意の LLM に対して**同一の測定器・同一の統計処理・同一の実行条件**で走らせ，比較可能な形で測定する Rust 製の研究基盤．

> Phase 1 完了．175 tests green，既定ビルドはネットワーク非依存（network-free）．

---

## 概要 / なぜ作るか

個々のベンチマークを走らせるツールは既に存在する（PyRIT 等）．本ライブラリの存在理由は別のところにある．**同一の測定器・統計処理・実行条件を全ベンチマークに強制すること**である．

既存のジェイルブレイク／プロンプトインジェクション・ベンチマークには構造的な欠陥がある（設計動機の詳細は本 README の下記各節が典拠）．これらは推測ではなく，上流リポジトリを実際に取得して実測した結果に基づく．

- **測定器（judge）が信用できない**．LLM-judge の再現率は 0.06〜0.65 と幅があり，judge 信頼性を数値で開示しないベンチマークが多い．**HarmBench 分類器の偽陽性率 FPR = 0.268 を独立に再現**した．人手多数決による「真の ASR」36.7% に対し，同分類器は 49.0%（1.34 倍に水増し）と報告する．`harmbench_cf` だけが他 judge 3 種と 77〜78% しか一致しない外れ値である．
- **指標・統計が貧弱**．単一設定の ASR 報告では不十分（bijection 攻撃は単一最良 ASR 81% に対し 36 variant の union coverage が 100%）．最大規模のベンチでも信頼区間を伴う報告がほとんど無い．拒否率を単独指標にしてはならない．
- **比較可能性が成立していない**．40 ベンチマーク横断で Kendall W = 0.10（p = 0.94，ランキングが事実上無相関）．環境種別（emulated / real executable）の違いは測定誤差ではなく系統的バイアスである．

実測で判明した具体的事実（本プロジェクトの根拠）:

- **AdvBench の「二重梱包」**: HarmBench は AdvBench を再梱包しているが，テキストは 520 件完全一致する一方，原典 `llm-attacks` にある `target` 列（`"Sure, here is ..."`；GCG の最適化目標，拒否文字列マッチ ASR が依拠する列）を**落としている**．同じテキストでも実行可能な測定が違う．本プロジェクトは原典 `llm-attacks` を AdvBench の正とする．
- **ベンチマークは互いに独立でない**: 完全一致による実測重複は JBB ∩ AdvBench = 11，JBB ∩ HarmBench = 9，AdvBench ∩ HarmBench = 0．「3 つのベンチが一致した」が同じ設問を数えているだけのことがある．自己申告の `Source` 列は当てにならない（`AdvBench` 申告 18 件のうち実際に一致するのは 11 件）．
- **「判定不能」が第 3 の状態として実在する**: HarmBench 分類器は yes/no 以外で `-1`，StrongREJECT v1 はパース失敗時に `NaN` を返す．0 に潰せば ASR は下振れ，成功に潰せば上振れするが，どちらも報告されない．**判定は三値**である．
- **「StrongREJECT の judge」が 2 つある**: 原典 `alexandrasouly/strongreject`（deprecated, v1）と後継 `dsbowen/strong_reject`（v2）で**ルーブリックのプロンプトが異なる**．採点式は代数的に同一だが，差は完全にプロンプト側にある．「StrongREJECT スコア 0.42」と書かれた 2 論文が，別の測定器の数字である可能性がある．

---

## 設計原則

- **三値判定** — 判定は `Success | Failure | Undecidable { reason }`．判定不能を成功／失敗に潰さず，二値への還元は集計側の明示的な選択とし，潰した件数を必ず併記する．
- **CaseId と ContentKey の分離** — 同一性（identity）は `(repo, commit, path, row)` の provenance と出自を含めた `CaseId` で決める（同一テキストでも出自が違えば別 Case）．重複検出（dedup）は出自を含めない内容フィンガープリント `ContentKey = hash(normalize(prompt), normalize(context))` で行う．dedup キーは非独立性の**報告にのみ**使い，Case は統合しない．
- **union coverage（単一設定 ASR の否定）** — ある Case に攻撃バリアント集合 `V` を当て，`coverage(case) = 1 iff ∃v∈V. verdict == Success`．変換は `Case` に焼き込まず，生成時に `(Case.prompt, AttackRef)` から最終プロンプトを導出する（`Case` は不変）．
- **多試行 ASR + 信頼区間** — Anthropic 式の 1 / 10 / 100 回開示．単発 ASR（二項割合）は **Wilson score 区間**を既定とし，Clopper–Pearson は最悪ケース報告用オプション，多試行曲線・union coverage・judge 間差分など単純割合でない統計は **Case 単位のブートストラップ（percentile / BCa）** で出す．`Undecidable` は分母から除外し件数を併記する．
- **network-free 既定** — 既定ビルドは HTTP を要さない．LLM プロバイダは cargo feature で gate し，既定は mock プロバイダ．正解データ（JBB `judge-comparison.csv`）を用いた judge 信頼性の再現は LLM を 1 回も呼ばずに実行・テストできる．
- **ハイブリッド型モノレポ** — 自作コードは単一リポジトリ（core と adapter のバージョン不整合が静かに数ポイント差を生むのを防ぐ）．上流ベンチの実体のみ `third_party/` に submodule で SHA 固定する．
- **有害データを vendoring しない** — 設問データは submodule 参照とし実体を持たない．per-source ライセンスが未定義のもの（MaliciousInstruct / HarmfulQ / OpenAI System Card 等）があるため，法的にも submodule 参照が正しい．

補足: **環境種別（`EnvKind`）と adapter 種別は別物**．`EnvKind`（`StaticPrompt` / `Emulated` / `RealExecutable`）はスコア比較の可否を決める科学的性質で第一級メタデータ，adapter 種別（`native` / `sidecar`）は実装都合．集計 API は「同一 `EnvKind` 内でのみ比較」を原則の宣言で終わらせず型で強制する．測定器（`InstrumentRef`）を跨いだ ASR の単純平均も既定で禁止する．

---

## アーキテクチャ / crate 構成

11 個の `pil-*` crate からなる Cargo workspace（`pil-` = **p**rompt-**i**njection-**l**ab）．

| crate | 役割 |
|---|---|
| `pil-core` | 型定義．`SourceRef`（同一性の単位 `(repo, commit, path, row)`）/ `Case`（`target: Option<String>` で AdvBench の二重梱包を型で表現）/ `EnvKind` / 三値 `Verdict` / `InstrumentRef` + `MeasurementParams` / `Measurement` / `Trial`（1 生成に測定器を複数ぶら下げる `measurements: Vec<Measurement>`）/ `AttackRef` + `Transform` |
| `pil-llm` | プロバイダ抽象（独立実装）．`LlmConfig` / `CallMetadata`（何と話したかを全呼び出しで記録）/ キャッシュ（`hash(rendered_prompt + model + params + attempt + seed)`；多試行が 1 件に潰れない）/ `top_logprobs` 公開．Ollama 既定 + OpenAI / Anthropic / Gemini を feature gate |
| `pil-metrics` | 内部を 3 分割．`instrument`（1 件ずつ判定；文字列マッチ / LLM 生成判定 / LLM logprob 判定 / ハッシュ照合の 4 種）・`aggregate`（ASR・信頼区間・union coverage・多試行 ASR，`EnvKind`/`InstrumentRef` で比較可能性を強制）・`reliability`（**判定器自身の再現率・FPR を測定**，第一級） |
| `pil-bench-advbench` | AdvBench ローダ（原典 `llm-attacks`，`goal,target` スキーマ，520 件） |
| `pil-bench-harmbench` | HarmBench ローダ（ファイルごとに異なる 5〜9 列可変スキーマ + Tags の `", "` 分解 + MinHash Jaccard copyright 判定 + cls テンプレート選択） |
| `pil-bench-strongreject` | StrongREJECT ローダ + ルーブリック **v1 / v2 を両方 native Rust 再実装**（同一応答への判定差を測る） |
| `pil-bench-jbb` | JBB ローダ（harmful 100 + benign 100 + `judge-comparison.csv` 300） |
| `pil-attacks` | `AttackRef` 変換レンダラ（Identity / Base64 / Leetspeak / Translate / Roleplay / RefusalSuppression）．**文献既存手法の再現のみ**，新規攻撃は作らない．`source: Option<SourceRef>` で再現元 provenance を刻む |
| `pil-runner` | 多試行・有界並行（semaphore）・token-bucket レート制御（429 は `Retry-After` 尊重 + 指数バックオフ）・中断再開（`(CaseId, instrument, attempt, seed)` 単位の append-only JSONL，再起動時に完了タプルをスキップ，atomic write/rename で冪等） |
| `pil-report` | run 成果物からの集計（信頼区間・undecidable 件数の併記，union coverage，多試行 ASR，過剰拒否，`ContentKey` による非独立性の自動検出） |
| `pil-cli` | `pil` バイナリ（`reliability` / `run` / `report`） |

上流ローダは native Rust で持ち，上流の Python ローダ（`main` ブランチや個人アカウントの raw URL をハードコード取得）は一切経由しない．これにより SHA 固定が実効化され，「どの SHA の何行目か」が全 `Case` に保証される．データの読み取り・正規化・`SourceRef` 付与はグルーであり，Rust に置いても測定値は変わらない．

Phase 2 で環境型ベンチ（AgentDojo 等）を取り込む際は**制御を反転**させ，Rust プロセスが OpenAI 互換のローカルエンドポイントを立て，Python 側ベンチの `base_url` をそこへ向ける．モデル呼び出しを 1 系統に集約して比較可能性の破綻を防ぐ．Phase 1 ではこのエンドポイントは不要（native adapter のみ）．

---

## セットアップ

上流ベンチの実体は `third_party/` の submodule で参照する（**実体はリポジトリに vendoring しない**）．clone 時は必ず submodule を取得する．

```bash
# 新規 clone
git clone --recursive git@github.com:akitenkrad/prompt-injection-lab.git

# 既存 clone に対して
git submodule update --init --recursive
```

submodule（すべて commit 固定，各々は上流のライセンスを保持）:

| submodule パス | 上流 | 提供データ | ライセンス |
|---|---|---|---|
| `third_party/llm-attacks` | `github.com/llm-attacks/llm-attacks` | AdvBench（`goal,target`，`target` 列あり） | MIT |
| `third_party/HarmBench` | `github.com/centerforaisafety/HarmBench` | HarmBench（text / contextual / copyright） | MIT |
| `third_party/strongreject` | `github.com/alexandrasouly/strongreject` | StrongREJECT 設問 + ルーブリック **v1** | MIT |
| `third_party/strong_reject` | `github.com/dsbowen/strong_reject` | StrongREJECT ルーブリック **v2**（provenance 用） | MIT |
| `third_party/JBB-Behaviors` | `huggingface.co/datasets/JailbreakBench/JBB-Behaviors` | JBB harmful 100 + benign 100 + judge-comparison 300 | MIT |

Rust toolchain: edition 2021，rust-version 1.80 以上（`rust-toolchain.toml` は channel 1.94.0 を固定）．

---

## ビルド & テスト

既定ビルドはネットワーク非依存で通る（HTTP を要するバックエンドは feature gate 済み）．

```bash
cargo build                    # 既定 = network-free
cargo test --workspace         # 175 tests
cargo clippy --workspace
cargo fmt --check
```

LLM プロバイダは `pil-llm` の cargo feature で選ぶ（既定は無効，mock プロバイダはネットワーク不要）:

```bash
# 実 LLM を使う場合のみ有効化する
cargo build --features pil-llm/ollama      # ollama / openai / anthropic / gemini
```

- 既定 feature は空（`default = []`）．`net` を要する `ollama` / `openai` / `anthropic` / `gemini` は明示指定時のみ有効．
- Ollama バックエンドは `top_logprobs`（v0.12.11 で対応）のため `>= 0.12.11` を要求し，起動時にバージョン検査して満たさなければ明示的に失敗する．

---

## 使い方（CLI）

`pil` は 3 つのサブコマンドを持つ．成果物はタイムスタンプ付きの `results/{subcommand}_YYYYMMDD_HHMMSS/` に落ち，submodule pin・パラメータ・タイムスタンプを刻んだ `provenance.json` を必ず同梱する．グローバルオプション `--repo-root <PATH>` で `third_party/` を含むルートを指定できる（省略時は CWD から上方探索）．

### `pil reliability` — judge 信頼性の開示

LLM／ネットワーク不使用．JBB `judge-comparison.csv`（300 件，人手 3 名 + 多数決 + 分類器 4 種）を正解データとして，recall / FPR / precision / F1・報告 ASR と真の ASR の乖離（水増し倍率）・アノテータ間一致（Cohen's kappa，測定精度の上限）・判定器間の一致率を出す（HarmBench 分類器 FPR 0.268 の再現）．

```bash
cargo run -p pil-cli -- reliability
```

### `pil run` — suite 実行で Trial 生成（生成 + 測定）

suite に従い Case × Attack × attempt を生成し，各応答に複数の測定器を当てる．checkpoint（中断再開）と provenance を出力する．

```bash
cargo run -p pil-cli -- run --suite suites/phase1-smoke.toml --provider mock
```

- `--suite <PATH>`: suite TOML（`suites/phase1-smoke.toml` = 高速 E2E 最小 / `suites/phase1-full.toml` = 4 ベンチ全件）．
- `--provider <mock|ollama>`: suite の値を上書き（省略時は suite の値；`mock` は network-free）．
- 出力: `results/run_<TS>/` に `trials.jsonl` / `cases.jsonl` / `checkpoint.jsonl` / `run_meta.json` / `provenance.json`．

### `pil report` — run 成果物から集計

run ディレクトリから，単発 ASR + 信頼区間 / union coverage / asr@k（多試行）/ 過剰拒否（JBB benign）/ 非独立性（`ContentKey`）を集計する．

```bash
cargo run -p pil-cli -- report --run results/run_20260719_120000
```

---

## 責任ある利用 / データポリシー

本リポジトリは**防御的評価・安全性研究のための dual-use ツール**である．やらないこと（非目標）を明示する:

- **リーダーボードを作らない**．出力は常に測定器・環境種別・信頼区間つきの条件付き数値であり，単一順位ではない．
- **新規攻撃手法を研究・生成しない**．Phase 1〜2 は既存ベンチの実行・測定が目的で，mutator は既存手法の再現に限る．
- **安全性の認証・保証を与えない**．反証可能性の無い主張はマーケティング上の主張として扱う（自らにも適用）．
- **有害コンテンツを同梱・再配布しない**．設問データは submodule 参照とし実体を持たない．copyright 系は生テキストでなくハッシュのみを扱う．

生成される攻撃プロンプトと有害応答（`Measurement.raw` 等に残る）は測定・監査の目的でのみ扱い，リポジトリ外に公開しない．**有害データおよび応答キャッシュはコミットしない**（`.cache/` / `results/` は gitignore 対象）．

---

## Phase 計画

- **Phase 1（完了 — 今回）**: データセット型ベンチで縦串を通す．対象は AdvBench 520 / HarmBench 400（contextual 100・copyright 100）/ StrongREJECT 313 + small 60（rubric v1・v2 両実装）/ JBB harmful 100 + benign 100 + judge-comparison 300．差別化点として **測定器の信頼性開示**・**単一設定 ASR の否定（union coverage）**・**多試行 ASR と信頼区間**・**ベンチ間非独立性の自動検出**・**過剰拒否**を実証する．4 ベンチが全て `StaticPrompt` のため，環境種別を跨ぐ比較は Phase 2 を待つ．
- **Phase 2**: 環境型（AgentDojo 等）を sidecar で取り込み，OpenAI 互換シムを実装．**環境種別を跨いだ横断比較**がここで初めて成立する．fine-tuned judge（`qylu4156/strongreject-15k-v1`，logit 期待値式）を追加．
- **Phase 3（候補）**: 適応型ベンチマークの標準形，マルチエージェント特有の創発的リスク（感染型 jailbreak・秘密結託・責任希薄化），AdvBench 近傍重複の埋め込みによる定量化．

---

## ライセンス

MIT（workspace 全体）．`third_party/` 配下の上流 submodule は各々のライセンス（いずれも MIT）を保持する．

---
*This file was generated by Claude Code.*
