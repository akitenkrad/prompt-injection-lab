//! リポジトリルートの解決と provenance 収集（DESIGN §7.1 / §7.3 / IMPLEMENTATION_PLAN M10）．
//!
//! ローダは `third_party/` の固定 SHA 実ファイルを直読する（§7.3）ため，`third_party/` を含む
//! リポジトリルートを確定する必要がある．CLI フラグ `--repo-root` 優先，無ければ CWD から上方に
//! `third_party/JBB-Behaviors` を探索する．submodule の固定 SHA は各 `pil-bench-*` crate の
//! `COMMIT` 定数（submodule pin と一致，§7.1）から転記し，provenance として成果物へ同梱する．

use std::path::{Path, PathBuf};

use serde::Serialize;

/// `third_party/` を含むリポジトリルートを解決する．
///
/// `explicit` が与えられればそれを使い，無ければ CWD から上方へ `third_party/JBB-Behaviors`
/// が存在するディレクトリを探す．見つからなければ CWD を返す（loader が明示エラーにする）．
pub fn resolve_repo_root(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut dir: &Path = &cwd;
    loop {
        if dir.join("third_party/JBB-Behaviors").is_dir() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    cwd
}

/// submodule 1 本の pin 情報（§7.1）．
#[derive(Debug, Clone, Serialize)]
pub struct SubmodulePin {
    /// 上流リポジトリ識別子（例: `centerforaisafety/HarmBench`）．
    pub upstream: &'static str,
    /// 固定 SHA（submodule pin と一致）．
    pub commit: &'static str,
}

/// Phase 1 で参照する上流 submodule の pin 一覧（§7.1）．
///
/// 各 `pil-bench-*` crate の `COMMIT` / `UPSTREAM` 定数を単一の出所として転記する．
pub fn submodule_pins() -> Vec<SubmodulePin> {
    vec![
        SubmodulePin {
            upstream: pil_bench_advbench::UPSTREAM,
            commit: pil_bench_advbench::COMMIT,
        },
        SubmodulePin {
            upstream: pil_bench_harmbench::UPSTREAM,
            commit: pil_bench_harmbench::COMMIT,
        },
        SubmodulePin {
            upstream: pil_bench_strongreject::UPSTREAM,
            commit: pil_bench_strongreject::COMMIT,
        },
        SubmodulePin {
            upstream: pil_bench_jbb::UPSTREAM,
            commit: pil_bench_jbb::COMMIT,
        },
    ]
}
