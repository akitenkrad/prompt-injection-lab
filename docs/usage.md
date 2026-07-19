**English** | [日本語](usage.ja.md)

# Usage

## Submodules

Upstream benchmark bodies are referenced under `third_party/` as SHA-pinned Git submodules — **the data is referenced, not vendored** into this repository. Each submodule retains its upstream license (all MIT).

| submodule path | upstream | data provided | version |
|---|---|---|---|
| `third_party/llm-attacks` | `github.com/llm-attacks/llm-attacks` | AdvBench (`goal,target`, with the `target` column) | — |
| `third_party/HarmBench` | `github.com/centerforaisafety/HarmBench` | HarmBench (text / contextual / copyright) | — |
| `third_party/strongreject` | `github.com/alexandrasouly/strongreject` | StrongREJECT questions + rubric **v1** | v1 |
| `third_party/strong_reject` | `github.com/dsbowen/strong_reject` | StrongREJECT rubric **v2** (for provenance) | v2 |
| `third_party/JBB-Behaviors` | `huggingface.co/datasets/JailbreakBench/JBB-Behaviors` | JBB harmful 100 + benign 100 + judge-comparison 300 | — |
| `third_party/agentdojo` | `github.com/ethz-spylab/agentdojo` | AgentDojo environments / tools / scoring | v0.1.35 |

Clone with submodules:

```bash
# Clone with submodules
git clone --recurse-submodules git@github.com:akitenkrad/prompt-injection-lab.git

# Or fetch submodules after a regular clone
git submodule update --init --recursive
```

Rust toolchain: edition 2021, rust-version 1.80 or newer (`rust-toolchain.toml` pins channel 1.94.0).

## Build and test

The default build is network-free — every HTTP-requiring backend is behind a cargo feature.

```bash
cargo build                    # default = network-free
cargo test --workspace
cargo clippy --workspace
cargo fmt --check
```

### Cargo features

The default build requires no network. Backends and sidecar plumbing are opt-in:

- `pil-llm`: `ollama` (default backend), `openai`, `anthropic`, `gemini` — the LLM providers. The default feature set is empty (`default = []`); the mock provider needs no network.
- `pil-shim`: `shim` — brings in the axum/tokio HTTP server for the OpenAI-compatible endpoint. Without it, only the pure OpenAI ⇄ `pil-llm` mapping is compiled.
- `pil-sidecar`: `sidecar` — enables the actual Python process launch (`tokio::process`).
- `pil-bench-agentdojo`: `agentdojo` — gates the sidecar-driven live path (requires a real AgentDojo pip install and a tool-capable model), documented as `#[ignore]` integration tests and kept out of default CI.

```bash
# Enable a real LLM backend only when you need one
cargo build --features pil-llm/ollama
```

The Ollama backend requires `>= 0.12.11` (which added `top_logprobs`) and checks the version at startup, failing explicitly if the requirement is not met.

## CLI

`pil` has three subcommands. Artifacts land in a timestamped `results/{subcommand}_YYYYMMDD_HHMMSS/`, always accompanied by a `provenance.json` recording the submodule pins, parameters, and timestamp. The global option `--repo-root <PATH>` selects the root containing `third_party/` (defaults to an upward search from the current directory).

### `pil reliability` — disclose judge reliability

No LLM, no network. Using JBB `judge-comparison.csv` (300 items: 3 human annotators + majority vote + 4 classifiers) as ground truth, it reports recall / FPR / precision / F1, the gap between reported and true ASR (inflation factor), inter-annotator agreement (Cohen's kappa, the ceiling on measurement precision), and inter-judge agreement rates — reproducing the HarmBench classifier's FPR of 0.268.

```bash
cargo run -p pil-cli -- reliability
```

### `pil run` — generate Trials from a suite (generate + measure)

Following a suite, it generates Case × Attack × attempt and applies multiple instruments to each response, emitting a checkpoint (resume-from-interrupt) and provenance.

```bash
cargo run -p pil-cli -- run --suite suites/phase1-smoke.toml --provider mock
```

- `--suite <PATH>`: the suite TOML (`suites/phase1-smoke.toml` = fast minimal E2E; `suites/phase1-full.toml` = all four benchmarks in full).
- `--provider <mock|ollama>`: overrides the suite's value (defaults to the suite's; `mock` is network-free).
- Output: `results/run_<TS>/` with `trials.jsonl` / `cases.jsonl` / `checkpoint.jsonl` / `run_meta.json` / `provenance.json`.

### `pil report` — aggregate from run artifacts

From a run directory, it aggregates single-shot ASR + confidence interval, union coverage, asr@k (multi-trial), over-refusal (JBB benign), and non-independence (`ContentKey`).

```bash
cargo run -p pil-cli -- report --run results/run_20260719_120000
```
