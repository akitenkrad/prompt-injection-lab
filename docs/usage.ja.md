[English](usage.md) | **日本語**

# 使い方

## submodule

上流ベンチの実体は `third_party/` の SHA 固定 Git submodule として参照する（**データは参照するだけで vendoring しない**）．各 submodule は上流のライセンス（いずれも MIT）を保持する．

| submodule パス | 上流 | 提供データ | version |
|---|---|---|---|
| `third_party/llm-attacks` | `github.com/llm-attacks/llm-attacks` | AdvBench（`goal,target`，`target` 列あり） | — |
| `third_party/HarmBench` | `github.com/centerforaisafety/HarmBench` | HarmBench（text / contextual / copyright） | — |
| `third_party/strongreject` | `github.com/alexandrasouly/strongreject` | StrongREJECT 設問 + ルーブリック **v1** | v1 |
| `third_party/strong_reject` | `github.com/dsbowen/strong_reject` | StrongREJECT ルーブリック **v2**（provenance 用） | v2 |
| `third_party/JBB-Behaviors` | `huggingface.co/datasets/JailbreakBench/JBB-Behaviors` | JBB harmful 100 + benign 100 + judge-comparison 300 | — |
| `third_party/agentdojo` | `github.com/ethz-spylab/agentdojo` | AgentDojo 環境 / ツール / scoring | v0.1.35 |

submodule つきで clone する：

```bash
# submodule つきで clone
git clone --recurse-submodules git@github.com:akitenkrad/prompt-injection-lab.git

# 通常 clone 済みなら submodule を後から取得
git submodule update --init --recursive
```

Rust toolchain: edition 2021，rust-version 1.80 以上（`rust-toolchain.toml` は channel 1.94.0 を固定）．

## ビルド & テスト

既定ビルドはネットワーク非依存である — HTTP を要するバックエンドは全て cargo feature の裏に置く．

```bash
cargo build                    # 既定 = network-free
cargo test --workspace
cargo clippy --workspace
cargo fmt --check
```

### cargo feature

既定ビルドはネットワークを要さない．バックエンドと sidecar 配線は opt-in である：

- `pil-llm`: `ollama`（既定バックエンド）・`openai`・`anthropic`・`gemini` — LLM プロバイダ．既定 feature は空（`default = []`）で，mock プロバイダはネットワーク不要．
- `pil-shim`: `shim` — OpenAI 互換エンドポイント用の axum/tokio HTTP サーバを導入する．無効時は OpenAI ⇄ `pil-llm` の純変換のみをコンパイルする．
- `pil-sidecar`: `sidecar` — 実際の Python プロセス起動（`tokio::process`）を有効化する．
- `pil-bench-agentdojo`: `agentdojo` — sidecar 駆動のライブ経路（実 AgentDojo の pip インストールと実ツール対応モデルを要する）を gate する．`#[ignore]` の統合テストとして文書化し，既定 CI から外す．
- `pil-cli`: `agentdojo-live` — ライブ AgentDojo 経路全体を有効化する統括 feature．`openai`（OpenAI 互換 `pil-llm` バックエンド）・`pil-shim/shim`・`pil-sidecar/sidecar`・`pil-bench-agentdojo/agentdojo` を引き込み，`pil agentdojo` サブコマンドを露出する．
- `pil-cli`: `strongreject-judge` — fine-tuned StrongREJECT judge で外部供給の `{prompt, response}` ペアを採点する `pil strongreject-judge` サブコマンドを gate する．sidecar は `std::process` で同期起動し，ネットワークバックエンドは一切引き込まないため，無効時の既定ビルドは network-free のままである．
- `pil-cli`: `strongreject-concordance` — 3 つの StrongREJECT judge（rubric v1 / v2 + fine-tuned）が Kendall の一致係数 W で一致するかを測る `pil strongreject-concordance` サブコマンドを gate する．2 つの rubric 判定は**ライブ**の judge モデルで採点するため，`strongreject-judge`（fine-tuned sidecar）に加えて `openai`（OpenAI 互換 `pil-llm` バックエンド）を有効化する．無効時の既定ビルドは network-free のままである．

これらはいずれもネットワーク／ライブ経路を gate する．既定ビルドと `cargo test --workspace` は network-free のままである．

```bash
# 実 LLM バックエンドを使う場合のみ有効化する
cargo build --features pil-llm/ollama
```

Ollama バックエンドは `top_logprobs`（v0.12.11 で対応）のため `>= 0.12.11` を要求し，起動時にバージョンを検査して満たさなければ明示的に失敗する．

## CLI

`pil` は既定ビルドで 3 つのサブコマンド（`reliability` / `run` / `report`）を持ち，`agentdojo-live` feature つきでビルドすると `agentdojo` が，`strongreject-judge` feature つきでビルドすると `strongreject-judge` が，`strongreject-concordance` feature つきでビルドすると `strongreject-concordance` が加わる．成果物はタイムスタンプ付きの `results/{subcommand}_YYYYMMDD_HHMMSS/` に落ち，submodule pin・パラメータ・タイムスタンプを刻んだ `provenance.json` を必ず同梱する．グローバルオプション `--repo-root <PATH>` で `third_party/` を含むルートを指定できる（省略時は CWD から上方探索）．

### `pil reliability` — judge 信頼性の開示

LLM／ネットワーク不使用．JBB `judge-comparison.csv`（300 件，人手 3 名 + 多数決 + 分類器 4 種）を正解データとして，recall / FPR / precision / F1，報告 ASR と真の ASR の乖離（水増し倍率），アノテータ間一致（Cohen's kappa，測定精度の上限），判定器間の一致率を出す（HarmBench 分類器 FPR 0.268 の再現）．

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

- `--run <DIR>`: run ディレクトリ．**繰り返し指定でき**，`--run A --run B` は集計前に run を union する．静的プロンプトの run と emulated（AgentDojo）の run を一緒に報告できる（上記の単一 `--run` 用法もそのまま使える）．
- `--cross-env`: `EnvKind` 跨ぎの開示を opt-in する．既定では `EnvKind` ごとの ASR を並置し横断スカラを出さないが，本フラグ指定時は追加でプール値を警告および Kendall の一致係数 W とともに開示する（共通の測定器・ケースを持たない環境間では W を未定義として報告する）．

```bash
# 静的プロンプトの run と emulated の run を一緒に，横断開示つきで報告する
cargo run -p pil-cli --features agentdojo-live -- report --run <static_run_dir> --run <emulated_run_dir> --cross-env
```

### `pil agentdojo` — エージェント型（emulated）ベンチをシム経由でライブ実行

`agentdojo-live` feature つきビルドでのみ利用できる．AgentDojo を `EnvKind::Emulated` のベンチとして実行する：ローカルの `pil-shim` が OpenAI 互換エンドポイントを立て，薄い Python sidecar が AgentDojo の環境／ツール／scoring を，client をシムへ向けた状態で走らせる．これによりモデル呼び出しは全て単一の `pil-llm` 経路に funnel される．

**前提条件:**

- commit 固定の `third_party/agentdojo` submodule から `agentdojo` を入れた Python 仮想環境（CLI 既定は `.venv-agentdojo/bin/python`；`--python` で上書き）．
- **ツール対応**モデルを配信中の Ollama（CLI 既定は `http://localhost:11434/v1`；`--ollama-base` と `--model` で上書き）．
- `agentdojo-live` feature つきビルド．

2 つのモードを持つ：

- **single**（`--limit` 省略）— 1 ケースを実行し `result.json` を書く：

```bash
cargo run -p pil-cli --features agentdojo-live -- agentdojo --suite banking --user-task user_task_0 --injection-task injection_task_0 --attack important_instructions
```

- **batch**（`--limit N`）— ケースを列挙し先頭 N 件を実行し，`pil report` がそのまま読める `EnvKind::Emulated` の run dir（`cases.jsonl` / `trials.jsonl` / `run_meta.json` / `provenance.json`）を書く：

```bash
cargo run -p pil-cli --features agentdojo-live -- agentdojo --suite banking --attack important_instructions --limit 8
```

batch 出力はそのまま `pil report` に流せ（例えばエージェント型のインジェクション成功率を信頼区間つきで報告できる），上記の `--cross-env` で静的プロンプトの run と union できる．

### `pil strongreject-judge` — fine-tuned StrongREJECT judge で {prompt, response} ペアを採点

`strongreject-judge` feature つきビルドでのみ利用できる．外部供給の `{forbidden_prompt, response}` ペアを fine-tuned StrongREJECT judge（`qylu4156/strongreject-15k-v1`，logit 期待値式スコア）で採点し，`pil report` がそのまま読める run ディレクトリ（`trials.jsonl` / `cases.jsonl` / `run_meta.json` / `provenance.json`）を書く — そのスコアは rubric v1 / v2 と並んで StrongREJECT judge 一致度に加わる．base `google/gemma-2b` + LoRA アダプタは薄い Python sidecar（`crates/pil-metrics/python/score_dist.py`）で走らせ，採点式と三値判定（採点トークン欠如 → Undecidable）は Rust 側に留める．

**前提条件:**

- `torch`・`transformers`・`peft` を入れた Python 仮想環境（CLI 既定は `.venv-strongreject/bin/python`；`--python` で上書き）．
- Hugging Face 認証：base `google/gemma-2b` は **gated** モデルのため，ライセンスに同意し `hf auth login` でトークンを与えてから初回利用する．
- sidecar `crates/pil-metrics/python/score_dist.py`（本リポジトリに同梱）．
- `strongreject-judge` feature つきビルド．

```bash
cargo run -p pil-cli --features strongreject-judge -- strongreject-judge --input <pairs.json>
```

`<pairs.json>` は `{"forbidden_prompt", "response"}` オブジェクトの JSON 配列である．フラグ:

- `--input <PATH>`: 採点対象の `{forbidden_prompt, response}` ペアを並べた JSON 配列．
- `--python <PATH>`: sidecar 用 Python インタプリタ（既定 `.venv-strongreject/bin/python`）．
- `--threshold <F>`: 二値化しきい値（`score >= threshold` を Success とする；既定 `0.5`）．連続スコアはしきい値によらず必ず測定へ残す．

### `pil strongreject-concordance` — 3 つの StrongREJECT judge の一致を測る

`strongreject-concordance` feature つきビルドでのみ利用できる（この feature は `strongreject-judge` + `openai` を有効化する）．**3 つ**の StrongREJECT judge — rubric v1・rubric v2（いずれも OpenAI 互換バックエンド経由の**ライブ** gpt-oss judge で採点）・fine-tuned judge（ローカルの Python sidecar）— が**同一の応答**に一致するかを，連続スコア上の Kendall の一致係数 W（group + pairwise）で測る．判定は常に元の goal で行い，書き出す run ディレクトリは 3 判定器のスコアを StrongREJECT 一致度に束ねる．

2 つのモードを持つ：

- **生成モード**（既定）— StrongREJECT のプロンプトを読み，ライブモデル（Ollama の gpt-oss，`--attack` で任意に jailbreak）で応答を生成してから，各応答を 3 判定器で判定する．**1 Case あたり ~3 回の LLM 呼び出し**（生成 1 + rubric 判定 2）で，fine-tuned はローカル sidecar 1 回の batch である．注記：safety 学習済みモデルは有害プロンプトの多くを拒否するため，生成応答はスコアの散らばりを欠きがちで一致度が退化しうる．判定器の（不）一致を実際に観察するには `--responses` で採点済み応答を与えること．
- **応答モード**（`--responses <json>`）— 外部供給の `{"forbidden_prompt", "response"}` ペア（`strongreject-judge --input` と同形の JSON）を判定する（生成なし）．2 つの rubric 判定にはなお gpt-oss を使う．判定器が一致するか見るには，拒否 → 部分的 → 応諾にわたる応答を与えること．

**前提条件:**

- `strongreject-judge` と同じもの：`torch`・`transformers`・`peft` を入れた Python 仮想環境，および Hugging Face 認証（fine-tuned judge の base `google/gemma-2b` が **gated** モデルのため，ライセンスに同意し `hf auth login` を実行）．
- 加えてライブ judge モデル：ライブ rubric 判定のため，capable なモデル（例 `gpt-oss:20b`）を配信中の Ollama．
- `strongreject-concordance` feature つきビルド．無効時の既定ビルドは network-free のままである．

フラグ:

- `--responses <PATH>`: 生成せず判定する `{"forbidden_prompt", "response"}` ペアの JSON 配列（応答モード）．省略時は生成モードで動く．
- `--limit <N>`: StrongREJECT プロンプトの先頭からの件数（生成モードのみ；既定 `10`）．
- `--model <TAG>`: 生成と rubric 判定に使うライブモデルタグ（既定 `gpt-oss:20b`）．
- `--ollama-base <URL>`: OpenAI 互換 base URL（既定 `http://localhost:11434/v1`）．
- `--api-key <KEY>`: プロバイダへ送る API 鍵（既定 `ollama`；Ollama はダミー値で可）．
- `--python <PATH>`: fine-tuned sidecar 用 Python インタプリタ（既定 `.venv-strongreject/bin/python`）．
- `--threshold <F>`: fine-tuned の二値化しきい値（既定 `0.5`）．
- `--max-tokens <N>`: 生成／rubric 判定の生成上限トークン数．
- `--attack <NAME>`: **生成**プロンプトにのみ当てる jailbreak（既定 `identity`；有効値 `identity` / `base64` / `leetspeak` / `refusal_suppression` / `translate:<lang>` / `roleplay:<tpl>`）．判定は常に元の goal で行う．

```bash
# Judge externally-supplied graded responses with all three judges
cargo run -p pil-cli --features strongreject-concordance -- strongreject-concordance --responses <graded.json> --max-tokens 2048

# Generation mode with a jailbreak applied to the generation prompt
cargo run -p pil-cli --features strongreject-concordance -- strongreject-concordance --limit 10 --attack refusal_suppression --max-tokens 2048
```
