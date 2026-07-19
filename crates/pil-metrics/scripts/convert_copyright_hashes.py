#!/usr/bin/env python3
"""HarmBench copyright 参照ハッシュ (.pkl) → native JSON 中間形式への one-shot 変換.

DESIGN §8.2 / §11.4 / IMPLEMENTATION_PLAN.md M3「注」:
    Rust 実行時に Python pickle 依存を持ち込まないため, `datasketch.MinHash` のリストを
    pickle 化した参照ハッシュを, 事前に JSON へ変換しておく.

入力:
    third_party/HarmBench/data/copyright_classifier_hashes/{BehaviorID}.pkl
        各ファイルは `list[datasketch.MinHash]` (参照テキストのスライディングウィンドウ MinHash 群).

出力:
    <out-dir>/{BehaviorID}.json
        {
          "behavior_id": "<BehaviorID>",
          "num_perm": 128,
          "hashes": [[<u32>, ...], ...]   # 各要素が 1 MinHash の hashvalues
        }
    さらに datasketch との座標系互換 (実運用で出力側 MinHash と突き合わせるため) が必要なら
    --emit-permutations で共有置換テーブル permutations.json も書き出す:
        { "num_perm": 128, "permutations": [[a, b], ...] }

USAGE:
    python convert_copyright_hashes.py \
        --pkl-dir third_party/HarmBench/data/copyright_classifier_hashes \
        --out-dir crates/pil-metrics/data/copyright_classifier_hashes \
        [--emit-permutations]

依存: datasketch, numpy (前処理時のみ. Rust 実行時には不要).
"""

import argparse
import json
import pickle
import sys
from pathlib import Path


def minhash_to_hashvalues(mh) -> list[int]:
    """datasketch.MinHash の hashvalues を Python int のリストへ変換する."""
    return [int(v) for v in mh.hashvalues]


def convert_one(pkl_path: Path, out_dir: Path) -> int:
    behavior_id = pkl_path.stem
    # NOTE(security): pickle.load はローカルに commit 固定された HarmBench 参照ハッシュ
    # (third_party submodule, 信頼済み) のみを対象とする one-shot 前処理. 外部入力は読まない.
    # このスクリプトの目的自体が「pickle を Rust 実行時から排除する」ことにある (§8.2 注).
    with open(pkl_path, "rb") as f:
        reference_minhashes = pickle.load(f)  # noqa: S301

    hashes = [minhash_to_hashvalues(mh) for mh in reference_minhashes]
    num_perm = len(hashes[0]) if hashes else 0

    entry = {
        "behavior_id": behavior_id,
        "num_perm": num_perm,
        "hashes": hashes,
    }
    out_path = out_dir / f"{behavior_id}.json"
    with open(out_path, "w") as f:
        json.dump(entry, f)
    return len(hashes)


def emit_permutations(sample_pkl: Path, out_dir: Path) -> None:
    """任意: datasketch の置換テーブル (a, b) を共有ファイルへ書き出す.

    全 MinHash(num_perm=128, seed=1) は同一の置換テーブルを持つため, 1 サンプルから抽出できる.
    Rust 側 `MinHasher::from_permutations` に与えると出力 MinHash が pkl 参照と同じ座標系になる.
    """
    with open(sample_pkl, "rb") as f:
        mhs = pickle.load(f)
    if not mhs:
        return
    a_arr, b_arr = mhs[0].permutations  # numpy uint64 の (2, num_perm)
    permutations = [[int(a), int(b)] for a, b in zip(a_arr, b_arr)]
    out = {"num_perm": len(permutations), "permutations": permutations}
    with open(out_dir / "permutations.json", "w") as f:
        json.dump(out, f)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--pkl-dir", required=True, type=Path)
    ap.add_argument("--out-dir", required=True, type=Path)
    ap.add_argument("--emit-permutations", action="store_true")
    args = ap.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)
    pkls = sorted(args.pkl_dir.glob("*.pkl"))
    if not pkls:
        print(f"no .pkl found under {args.pkl_dir}", file=sys.stderr)
        return 1

    total = 0
    for pkl in pkls:
        n = convert_one(pkl, args.out_dir)
        total += 1
        print(f"{pkl.name}: {n} minhashes -> {pkl.stem}.json")

    if args.emit_permutations:
        emit_permutations(pkls[0], args.out_dir)
        print("wrote permutations.json")

    print(f"converted {total} behaviors -> {args.out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
