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
- `pil-cli`: `agentdojo-live` — the umbrella feature that turns on the whole live AgentDojo path. It pulls in `openai` (the OpenAI-compatible `pil-llm` backend), `pil-shim/shim`, `pil-sidecar/sidecar`, and `pil-bench-agentdojo/agentdojo`, and is what exposes the `pil agentdojo` subcommand.
- `pil-cli`: `strongreject-judge` — gates the `pil strongreject-judge` subcommand, which scores externally-supplied `{prompt, response}` pairs with the fine-tuned StrongREJECT judge. It launches a Python sidecar synchronously via `std::process` and pulls in no network backend, so the default build stays network-free without it.
- `pil-cli`: `strongreject-concordance` — gates the `pil strongreject-concordance` subcommand, which measures whether the three StrongREJECT judges (rubric v1 / v2 + fine-tuned) agree via Kendall's W. It activates `strongreject-judge` (the fine-tuned sidecar) plus `openai` (the OpenAI-compatible `pil-llm` backend), because the two rubric judges are graded by a **live** judge model; the default build stays network-free without it.

Every one of these gates a networked or live path; the default build and `cargo test --workspace` stay network-free.

```bash
# Enable a real LLM backend only when you need one
cargo build --features pil-llm/ollama
```

The Ollama backend requires `>= 0.12.11` (which added `top_logprobs`) and checks the version at startup, failing explicitly if the requirement is not met.

## CLI

`pil` has three subcommands in the default build (`reliability` / `run` / `report`), plus `agentdojo` when built with the `agentdojo-live` feature, `strongreject-judge` when built with the `strongreject-judge` feature, and `strongreject-concordance` when built with the `strongreject-concordance` feature. Artifacts land in a timestamped `results/{subcommand}_YYYYMMDD_HHMMSS/`, always accompanied by a `provenance.json` recording the submodule pins, parameters, and timestamp. The global option `--repo-root <PATH>` selects the root containing `third_party/` (defaults to an upward search from the current directory).

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

- `--run <DIR>`: the run directory. It is **repeatable** — passing `--run A --run B` unions the runs before aggregating, so a static-prompt run and an emulated (AgentDojo) run can be reported together. The default single-`--run` usage above still works.
- `--cross-env`: opt in to the cross-`EnvKind` disclosure. By default the report places per-`EnvKind` ASR side by side and emits no cross-environment scalar; with this flag it additionally discloses the pooled value with a warning plus Kendall's W (coefficient of concordance), which is reported as undefined when the environments share no common instrument or cases.

```bash
# Report a static-prompt run and an emulated run together, with the cross-environment disclosure
cargo run -p pil-cli --features agentdojo-live -- report --run <static_run_dir> --run <emulated_run_dir> --cross-env
```

### `pil agentdojo` — run an agentic (emulated) benchmark live through the shim

Available only when built with the `agentdojo-live` feature. It runs AgentDojo as an `EnvKind::Emulated` benchmark: a local `pil-shim` serves an OpenAI-compatible endpoint, and a thin Python sidecar runs AgentDojo's environment / tools / scoring with its client pointed at the shim, so every model call funnels through the single `pil-llm` path.

**Prerequisites:**

- A Python virtualenv with `agentdojo` installed from the pinned `third_party/agentdojo` submodule (the CLI defaults to `.venv-agentdojo/bin/python`; override with `--python`).
- A running Ollama serving a **tool-capable** model (the CLI defaults to `http://localhost:11434/v1`; override with `--ollama-base` and `--model`).
- Building with the `agentdojo-live` feature.

It has two modes:

- **single** (no `--limit`) — run one case and write a `result.json`:

```bash
cargo run -p pil-cli --features agentdojo-live -- agentdojo --suite banking --user-task user_task_0 --injection-task injection_task_0 --attack important_instructions
```

- **batch** (`--limit N`) — enumerate cases and run the first N, writing an `EnvKind::Emulated` run directory (`cases.jsonl` / `trials.jsonl` / `run_meta.json` / `provenance.json`) that `pil report` reads directly:

```bash
cargo run -p pil-cli --features agentdojo-live -- agentdojo --suite banking --attack important_instructions --limit 8
```

The batch output feeds straight into `pil report` (for example an agentic injection-success rate reported with a confidence interval), and can be unioned with a static-prompt run under `--cross-env` as shown above.

### `pil strongreject-judge` — score {prompt, response} pairs with the fine-tuned StrongREJECT judge

Available only when built with the `strongreject-judge` feature. It scores externally-supplied `{forbidden_prompt, response}` pairs with the fine-tuned StrongREJECT judge (`qylu4156/strongreject-15k-v1`, a logit-expectation score) and writes a run directory (`trials.jsonl` / `cases.jsonl` / `run_meta.json` / `provenance.json`) that `pil report` reads directly — so its scores join rubric v1 / v2 in the StrongREJECT judge concordance. The base `google/gemma-2b` + LoRA adapter run in a thin Python sidecar (`crates/pil-metrics/python/score_dist.py`); the scoring math and the three-valued verdict (score-token-absent → Undecidable) stay in Rust.

**Prerequisites:**

- A Python virtualenv with `torch`, `transformers`, and `peft` installed (the CLI defaults to `.venv-strongreject/bin/python`; override with `--python`).
- Hugging Face authentication: the base `google/gemma-2b` is a **gated** model, so you must accept its license and provide a token via `hf auth login` before first use.
- The sidecar `crates/pil-metrics/python/score_dist.py` (shipped in this repository).
- Building with the `strongreject-judge` feature.

```bash
cargo run -p pil-cli --features strongreject-judge -- strongreject-judge --input <pairs.json>
```

`<pairs.json>` is a JSON array of `{"forbidden_prompt", "response"}` objects. Flags:

- `--input <PATH>`: the JSON array of `{forbidden_prompt, response}` pairs to score.
- `--python <PATH>`: the sidecar Python interpreter (default `.venv-strongreject/bin/python`).
- `--threshold <F>`: the binarization threshold (`score >= threshold` is Success; default `0.5`). The continuous score is always retained on the measurement regardless of the threshold.

### `pil strongreject-concordance` — measure whether the three StrongREJECT judges agree

Available only when built with the `strongreject-concordance` feature (which activates `strongreject-judge` + `openai`). It measures whether the **three** StrongREJECT judges — rubric v1, rubric v2 (both graded by a **live** gpt-oss judge through the OpenAI-compatible backend), and the fine-tuned judge (the local Python sidecar) — agree on the **same** response, by computing Kendall's W (group + pairwise) over their continuous scores. Judging always uses the original goal; the run directory it writes joins the three judges' scores in the StrongREJECT concordance.

It has two modes:

- **Generation mode** (default) — loads StrongREJECT prompts, generates a response with a live model (gpt-oss via Ollama, optionally jailbroken via `--attack`), then judges each response with all three judges. Roughly **3 LLM calls per case** (1 generation + 2 rubric judgments); the fine-tuned judge is a single local sidecar batch. Note: a safety-trained model refuses most harmful prompts, so generated responses often lack score spread and the concordance can degenerate — supply graded responses via `--responses` to actually observe judge (dis)agreement.
- **Responses mode** (`--responses <json>`) — judges externally-supplied `{"forbidden_prompt", "response"}` pairs (the same JSON shape as `strongreject-judge --input`); no generation. It still uses gpt-oss for the two rubric judges. Use it with responses spanning refusal → partial → compliant to see whether the judges agree.

**Prerequisites:**

- The same as `strongreject-judge`: a Python virtualenv with `torch`, `transformers`, and `peft`, and Hugging Face authentication because the fine-tuned judge's base `google/gemma-2b` is a **gated** model (accept its license and run `hf auth login`).
- Additionally a live judge model: a running Ollama serving a capable model (e.g. `gpt-oss:20b`) for the live rubric judging.
- Building with the `strongreject-concordance` feature. The default build stays network-free without it.

Flags:

- `--responses <PATH>`: a JSON array of `{"forbidden_prompt", "response"}` pairs to judge without generation (responses mode). When omitted, the subcommand runs in generation mode.
- `--limit <N>`: the number of StrongREJECT prompts from the top (generation mode only; default `10`).
- `--model <TAG>`: the live model tag used for generation and rubric judging (default `gpt-oss:20b`).
- `--ollama-base <URL>`: the OpenAI-compatible base URL (default `http://localhost:11434/v1`).
- `--api-key <KEY>`: the API key sent to the provider (default `ollama`; Ollama accepts a dummy value).
- `--python <PATH>`: the fine-tuned sidecar Python interpreter (default `.venv-strongreject/bin/python`).
- `--threshold <F>`: the fine-tuned binarization threshold (default `0.5`).
- `--max-tokens <N>`: the generation / rubric-judging token cap.
- `--attack <NAME>`: a jailbreak applied to the **generation** prompt only (default `identity`; valid: `identity` / `base64` / `leetspeak` / `refusal_suppression` / `translate:<lang>` / `roleplay:<tpl>`). Judging always uses the original goal.

```bash
# Judge externally-supplied graded responses with all three judges
cargo run -p pil-cli --features strongreject-concordance -- strongreject-concordance --responses <graded.json> --max-tokens 2048

# Generation mode with a jailbreak applied to the generation prompt
cargo run -p pil-cli --features strongreject-concordance -- strongreject-concordance --limit 10 --attack refusal_suppression --max-tokens 2048
```
