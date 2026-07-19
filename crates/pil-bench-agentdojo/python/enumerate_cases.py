#!/usr/bin/env python3
"""AgentDojo の security ケースを列挙する薄殻（DESIGN §4.1 native-first）．

`get_suites(BENCHMARK_VERSION)` で 4 suite を取り，各 suite の user_tasks × injection_tasks を
巡回して `{suite, user_task_id, injection_task_id}` の JSON 配列を stdout に出す．列挙は irreducible な
Python 部（タスク定義は agentdojo 本体の in-memory 環境にしか無い）．glue は Rust 側に集約する．

前提: agentdojo が pip インストール済みであること（既定テストでは実行しない）．
出力: stdout に JSON 配列のみ．診断は stderr へ．

使用例:
    python enumerate_cases.py --benchmark-version v1.2.2
"""

import argparse
import json
import sys

BENCHMARK_VERSION = "v1.2.2"


def main() -> int:
    parser = argparse.ArgumentParser(description="Enumerate AgentDojo security cases as JSON.")
    parser.add_argument("--benchmark-version", default=BENCHMARK_VERSION)
    args = parser.parse_args()

    # agentdojo の import は薄殻内に閉じる（Rust 側は依存しない）．
    from agentdojo.task_suite.load_suites import get_suites

    suites = get_suites(args.benchmark_version)
    cases: list[dict[str, str]] = []
    for suite_name, suite in suites.items():
        for user_task_id in suite.user_tasks:
            for injection_task_id in suite.injection_tasks:
                cases.append(
                    {
                        "suite": suite_name,
                        "user_task_id": user_task_id,
                        "injection_task_id": injection_task_id,
                    }
                )

    json.dump(cases, sys.stdout)
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
