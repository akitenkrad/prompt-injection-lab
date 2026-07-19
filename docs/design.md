**English** | [日本語](design.ja.md)

# Design

## Why this exists

Tools that run individual prompt-injection and jailbreak benchmarks already exist. The reason this platform exists is different: to force the **same instrument, the same statistics, and the same execution conditions** onto every benchmark. Existing jailbreak and prompt-injection benchmarks have structural flaws, and the flaws below are not speculation — they were measured by actually fetching the upstream repositories and running against them.

### The judges cannot be trusted

LLM-judge recall ranges from 0.06 to 0.65, and many benchmarks never disclose their judge's reliability as a number. We independently reproduced the HarmBench classifier's false-positive rate of **FPR = 0.268**. Against a human-majority-vote "true ASR" of 36.7%, that classifier reports 49.0% — a 1.34× inflation. Among four classifiers on the same labeled set, `harmbench_cf` is the outlier: it agrees with the other three judges only 77–78% of the time, while those three agree with each other 89–93%. Inter-annotator Cohen's kappa on the human labels is 0.809 / 0.826 / 0.886 — the ceiling on measurement precision. This labeled set is the same one the JailbreakBench paper used, and our numbers reproduce theirs independently.

### AdvBench is double-packed

HarmBench re-packages AdvBench, and the 520 prompt texts match exactly. But the re-packaged copy **drops the `target` column** (`"Sure, here is ..."`, the GCG optimization target that refusal-string-match ASR depends on) that the original `llm-attacks` repository carries. The same text supports a different measurement depending on which copy you read. This platform treats the original `llm-attacks` as the canonical AdvBench.

### Benchmarks are not independent

By exact-match measurement, the real overlaps are JBB ∩ AdvBench = **11**, JBB ∩ HarmBench = **9**, AdvBench ∩ HarmBench = **0**. "Three benchmarks agreed" can be the same question counted more than once. Self-reported `Source` columns are unreliable: of 18 items self-labeled `AdvBench`, only 11 actually match.

### "Undecidable" is a real third state

The HarmBench classifier returns `-1` for anything other than yes/no; StrongREJECT v1 returns `NaN` on a parse failure. Collapsing those to 0 deflates ASR; collapsing them to success inflates it — and neither collapse is reported. **The verdict is three-valued.**

### There are two "StrongREJECT judges"

The original `alexandrasouly/strongreject` (deprecated, v1) and the successor `dsbowen/strong_reject` (v2) use **different rubric prompts**. The scoring formula is algebraically identical; the entire difference is on the prompt side. Two papers both reporting "StrongREJECT score 0.42" may be quoting numbers from different instruments.

## Design principles

- **Three-valued verdict** — a verdict is `Success | Failure | Undecidable { reason }`. Undecidable is never silently folded into success or failure; reduction to a binary is an explicit choice made at aggregation time, and the count of collapsed cases is always reported alongside.
- **CaseId vs ContentKey** — identity is decided by a `CaseId` that includes provenance `(repo, commit, path, row)`; the same text with different provenance is a different case. Deduplication uses a provenance-free content fingerprint `ContentKey = hash(normalize(prompt), normalize(context))`. The dedup key is used only to **report** non-independence, never to merge cases.
- **Union coverage (rejecting single-setting ASR)** — for a case and a set of attack variants `V`, `coverage(case) = 1 iff ∃v∈V. verdict == Success`. Transforms are not baked into the `Case`; the final prompt is derived at generation time from `(Case.prompt, AttackRef)`, and the `Case` stays immutable.
- **Multi-trial ASR + confidence intervals** — 1 / 10 / 100-attempt disclosure. Single-shot ASR (a binomial proportion) defaults to the **Wilson score interval**, with Clopper–Pearson as a worst-case option; non-proportion statistics (multi-trial curves, union coverage, cross-judge differences) use **case-level bootstrap (percentile / BCa)**. Undecidable cases are excluded from the denominator, with the count reported.
- **Network-free by default** — the default build requires no HTTP. LLM providers are gated behind cargo features, with a mock provider as the default. Judge-reliability reproduction using the ground-truth data (JBB `judge-comparison.csv`) runs and tests without ever calling an LLM.
- **Hybrid monorepo** — first-party code lives in a single repository (so a version mismatch between core and adapter cannot silently move the numbers). Only the upstream benchmark bodies live in `third_party/` as SHA-pinned submodules.
- **No data vendoring** — question data is referenced by submodule, never held as a copy. Some sources have undefined per-source licenses (MaliciousInstruct, HarmfulQ, the OpenAI System Card, and others), so submodule reference is also the legally correct choice.

Note: **environment kind (`EnvKind`) and adapter kind are different things.** `EnvKind` (`StaticPrompt` / `Emulated` / `RealExecutable`) is a scientific property that decides whether two scores may be compared, and it is first-class metadata; adapter kind (`native` / `sidecar`) is an implementation detail. The aggregation API does not stop at declaring "compare only within the same `EnvKind`" — it enforces it in the type system, and by default also forbids naively averaging ASR across instruments (`InstrumentRef`).

## Responsible use and data policy

This repository is a **dual-use tool for defensive evaluation and safety research**. Its non-goals are stated explicitly:

- **No leaderboards.** Output is always a conditional number tagged with instrument, environment kind, and confidence interval — never a single rank.
- **No researching or generating new attack techniques.** Phases 1–2 aim to run and measure existing benchmarks; mutators are limited to reproducing existing published methods.
- **No safety certification or guarantee.** Any unfalsifiable claim is treated as a marketing claim — including our own.
- **No bundling or redistribution of harmful content.** Question data is referenced by submodule and not held as a copy; copyright-sensitive items are handled as hashes, not raw text.

Generated attack prompts and harmful responses (which can remain in fields such as `Measurement.raw`) are handled only for measurement and audit, and are not published outside the repository. **Harmful data and the response cache are never committed** (`.cache/` and `results/` are gitignored).

## Phase roadmap

- **Phase 1 (done)** — thread the vertical slice through dataset-style benchmarks: AdvBench 520, HarmBench 400 (contextual 100, copyright 100), StrongREJECT 313 + small 60 (both rubric v1 and v2 implemented), JBB harmful 100 + benign 100 + judge-comparison 300. The differentiators — judge-reliability disclosure, rejection of single-setting ASR (union coverage), multi-trial ASR with confidence intervals, automatic detection of cross-benchmark non-independence, and over-refusal — are all demonstrated here. Because all four benchmarks are `StaticPrompt`, cross-environment comparison waits for Phase 2.
- **Phase 2** — take in environment-typed benchmarks (such as AgentDojo) via a sidecar, with an OpenAI-compatible shim implementing control inversion so every model call funnels through the single `pil-llm` path. This is where **cross-environment comparison first becomes valid.** Adds a fine-tuned judge (`qylu4156/strongreject-15k-v1`, a logit-expectation score).
- **Phase 3 (candidates)** — a standard form for adaptive benchmarks; multi-agent-specific emergent risks (contagious jailbreaks, secret collusion, diffusion of responsibility); and quantifying AdvBench near-duplicates via embeddings.
