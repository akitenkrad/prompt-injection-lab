#!/usr/bin/env python3
"""薄い殻の Python sidecar スタブ（DESIGN §4.1 native-first）．

M3 ではツール本体を持たず，「注入された OPENAI_BASE_URL のシムへ chat completion を 1 回投げ，
応答本文を stdout に出す」だけの irreducible な最小殻．stdlib のみ（openai/pip 不要）を使い，
モデル呼び出しがシム経由で pil-llm 単一経路へ funnel されることを証明する．
"""

import json
import os
import sys
import urllib.request


def main() -> int:
    base_url = os.environ["OPENAI_BASE_URL"].rstrip("/")
    api_key = os.environ.get("OPENAI_API_KEY", "")
    url = f"{base_url}/chat/completions"

    payload = json.dumps(
        {
            "model": "mock/mock-1",
            "messages": [{"role": "user", "content": "hello from sidecar"}],
        }
    ).encode("utf-8")

    request = urllib.request.Request(
        url,
        data=payload,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {api_key}",
        },
        method="POST",
    )

    with urllib.request.urlopen(request, timeout=10) as response:
        body = json.loads(response.read().decode("utf-8"))

    content = body["choices"][0]["message"]["content"]
    sys.stdout.write(content)
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
