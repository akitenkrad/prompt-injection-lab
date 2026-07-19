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

```bash
# 実 LLM バックエンドを使う場合のみ有効化する
cargo build --features pil-llm/ollama
```

Ollama バックエンドは `top_logprobs`（v0.12.11 で対応）のため `>= 0.12.11` を要求し，起動時にバージョンを検査して満たさなければ明示的に失敗する．

## CLI

`pil` は 3 つのサブコマンドを持つ．成果物はタイムスタンプ付きの `results/{subcommand}_YYYYMMDD_HHMMSS/` に落ち，submodule pin・パラメータ・タイムスタンプを刻んだ `provenance.json` を必ず同梱する．グローバルオプション `--repo-root <PATH>` で `third_party/` を含むルートを指定できる（省略時は CWD から上方探索）．

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
