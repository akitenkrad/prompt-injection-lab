#!/usr/bin/env python3
"""StrongREJECT fine-tuned judge の薄殻（native-first）.

Loads the base google/gemma-2b + the public LoRA adapter qylu4156/strongreject-15k-v1
and, for each (forbidden_prompt, response), returns the raw logits of the score tokens
"1".."5" at the last position. The Rust side (pil-metrics `expected_score`) turns those
into softmax * linspace(0,1,5). This mirrors upstream `strong_reject/evaluate.py::
strongreject_finetuned` exactly (same model, same 5-token readout).

Only the model-specific logit extraction lives here (irreducible: transformers/peft).
The scoring math and provenance stay in Rust.

Input:  a JSON array of {"forbidden_prompt": str, "response": str}, read from the file given
        by --input, or from stdin when --input is omitted.
Output: a JSON array on stdout: [{"entries": [["1", logit], ..., ["5", logit]]}, ...]
        one object per input item, index-aligned. On a per-item failure the object is
        {"entries": [], "error": "<msg>"} so the batch never aborts.

Env: HF auth via ~/.cache/huggingface/token (hf auth login). google/gemma-2b is gated.
"""

import argparse
import json
import sys
from pathlib import Path

BASE_MODEL = "google/gemma-2b"
ADAPTER = "qylu4156/strongreject-15k-v1"
ADAPTER_REVISION = "4bd893d32390d2cace4f067dc2e3ef5294fd78a2"
POSSIBLE_SCORES = ("1", "2", "3", "4", "5")


def load_template() -> str:
    """Read the faithful `strongreject_finetuned` template from the pinned submodule."""
    root = Path(__file__).resolve().parents[3]
    path = (
        root
        / "third_party"
        / "strong_reject"
        / "strong_reject"
        / "eval_files"
        / "judge_templates.json"
    )
    with open(path, encoding="utf-8") as f:
        return json.load(f)["strongreject_finetuned"]


def main() -> int:
    import torch
    from peft import PeftModel
    from transformers import AutoModelForCausalLM, AutoTokenizer

    parser = argparse.ArgumentParser(description="StrongREJECT fine-tuned judge の 5 トークン logits を出す．")
    parser.add_argument("--input", help="入力 JSON ファイル（省略時は stdin）．")
    args = parser.parse_args()

    if args.input:
        with open(args.input, encoding="utf-8") as f:
            items = json.load(f)
    else:
        items = json.load(sys.stdin)
    template = load_template()

    # 判定器モデル（base + LoRA）を 1 回だけロードする．gemma-2b は gated（要 HF 認証）．
    tokenizer = AutoTokenizer.from_pretrained(ADAPTER, revision=ADAPTER_REVISION)
    if not tokenizer.pad_token:
        tokenizer.pad_token = tokenizer.eos_token
    base = AutoModelForCausalLM.from_pretrained(BASE_MODEL, torch_dtype=torch.float32)
    model = PeftModel.from_pretrained(base, ADAPTER, revision=ADAPTER_REVISION)
    model.eval()

    # スコアトークン "1".."5" の vocab id（存在しなければ per-item error にする）．
    score_ids = []
    for s in POSSIBLE_SCORES:
        tid = tokenizer.vocab.get(s)
        score_ids.append(tid)

    out = []
    for item in items:
        try:
            prompt = template.format(
                forbidden_prompt=item["forbidden_prompt"], response=item["response"]
            )
            enc = tokenizer(prompt, return_tensors="pt")
            with torch.no_grad():
                logits = model(
                    input_ids=enc["input_ids"].to(model.device),
                    attention_mask=enc["attention_mask"].to(model.device),
                ).logits[:, -1]
            entries = []
            for s, tid in zip(POSSIBLE_SCORES, score_ids):
                if tid is None:
                    continue
                entries.append([s, float(logits[0, tid].item())])
            out.append({"entries": entries})
        except Exception as e:  # noqa: BLE001 — 1 件の失敗で batch を止めない
            out.append({"entries": [], "error": str(e)})

    json.dump(out, sys.stdout)
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
