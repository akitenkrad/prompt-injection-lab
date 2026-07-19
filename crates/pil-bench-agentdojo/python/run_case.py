#!/usr/bin/env python3
"""AgentDojo の 1 security ケースを実行する薄殻（DESIGN §4.1 native-first）．

`(suite, user_task, injection_task, attack)` を benchmark API で 1 件だけ走らせ，決定論的述語の
`{utility, security, error}` を JSON で stdout に出す．モデル呼び出しは env 差し替えのみでシムへ向く:
`OPENAI_BASE_URL`（シム base_url）を openai SDK がそのまま読むため，モンキーパッチは不要（§4.1）．
ツール実行・環境・scoring は agentdojo 本体（in-memory）のまま = irreducible な Python 部．

前提:
    - agentdojo が pip インストール済み．
    - シムが OpenAI tool-calling（tools/tool_calls/developer role）に対応（M2'）．
    - 実ツール対応モデル（MockProvider は tool_calls を返せない）．
    これらが揃わない限り既定テストでは実行しない（Rust 側は #[ignore] のライブテストで配線のみ検証）．

出力: stdout に `{"utility": bool, "security": bool, "error": str | null}` の JSON のみ．

使用例:
    OPENAI_BASE_URL=http://127.0.0.1:PORT/v1 OPENAI_API_KEY=pil-shim \\
        python run_case.py --suite banking --user-task user_task_0 \\
        --injection-task injection_task_1 --attack important_instructions --model gpt-4o-2024-05-13
"""

import argparse
import json
import sys
import tempfile
from pathlib import Path

BENCHMARK_VERSION = "v1.2.2"


def main() -> int:
    parser = argparse.ArgumentParser(description="Run one AgentDojo security case; emit JSON.")
    parser.add_argument("--suite", required=True)
    parser.add_argument("--user-task", required=True)
    parser.add_argument("--injection-task", required=True)
    parser.add_argument("--attack", default="important_instructions")
    parser.add_argument("--model", required=True)
    parser.add_argument("--benchmark-version", default=BENCHMARK_VERSION)
    args = parser.parse_args()

    # agentdojo の import は薄殻内に閉じる．
    from agentdojo.agent_pipeline.agent_pipeline import AgentPipeline, PipelineConfig
    from agentdojo.attacks.attack_registry import load_attack
    from agentdojo.benchmark import benchmark_suite_with_injections
    from agentdojo.task_suite.load_suites import get_suites

    suite = get_suites(args.benchmark_version)[args.suite]
    pipeline = AgentPipeline.from_config(PipelineConfig(llm=args.model, model_id=args.model))
    attacker = load_attack(args.attack, suite, pipeline)

    with tempfile.TemporaryDirectory() as logdir:
        results = benchmark_suite_with_injections(
            pipeline,
            suite,
            attacker,
            user_tasks=(args.user_task,),
            injection_tasks=(args.injection_task,),
            logdir=Path(logdir),
            force_rerun=True,
            benchmark_version=args.benchmark_version,
        )

    key = (args.user_task, args.injection_task)
    out = {
        "utility": bool(results["utility_results"][key]),
        "security": bool(results["security_results"][key]),
        "error": None,
    }
    json.dump(out, sys.stdout)
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
