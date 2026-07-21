//! サブコマンド実装（DESIGN §10 / IMPLEMENTATION_PLAN M10）．
//!
//! `reliability` / `run` / `report` の 3 本．成果物は `results/{subcommand}_YYYYMMDD_HHMMSS/`
//! に落とし，`provenance.json`（submodule pin・パラメータ・タイムスタンプ）を必ず同梱する．

#[cfg(feature = "agentdojo-live")]
pub mod agentdojo;
pub mod reliability;
pub mod report;
pub mod run;
#[cfg(feature = "strongreject-concordance")]
pub mod strongreject_concordance;
#[cfg(feature = "strongreject-judge")]
pub mod strongreject_judge;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::repo::{submodule_pins, SubmodulePin};
use crate::timestamp::now_utc_compact;

/// `results/{subcommand}_YYYYMMDD_HHMMSS/` を作って返す（§11.3）．
pub fn make_results_dir(repo_root: &Path, subcommand: &str) -> Result<PathBuf> {
    let dir = repo_root
        .join("results")
        .join(format!("{subcommand}_{}", now_utc_compact()));
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("results ディレクトリを作成できません: {}", dir.display()))?;
    Ok(dir)
}

/// 成果物 provenance（DESIGN §7.1 / §5.1）．
#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    /// サブコマンド名．
    pub subcommand: String,
    /// 生成時刻（UTC，`YYYYMMDD_HHMMSS`）．
    pub timestamp: String,
    /// スイート名（reliability では None）．
    pub suite: Option<String>,
    /// 実行パラメータの自由記述（model / seed / attempts 等）．
    pub params: serde_json::Value,
    /// 上流 submodule の固定 SHA（§7.1）．
    pub submodules: Vec<SubmodulePin>,
}

impl Provenance {
    /// provenance を新規作成する．
    pub fn new(subcommand: &str, suite: Option<String>, params: serde_json::Value) -> Self {
        Self {
            subcommand: subcommand.to_string(),
            timestamp: now_utc_compact(),
            suite,
            params,
            submodules: submodule_pins(),
        }
    }

    /// `provenance.json` として書き出す．
    pub fn write(&self, dir: &Path) -> Result<()> {
        let path = dir.join("provenance.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)
            .with_context(|| format!("provenance を書けません: {}", path.display()))?;
        Ok(())
    }
}

/// 文字列を成果物ファイルへ書く小ヘルパ．
pub fn write_text(dir: &Path, name: &str, contents: &str) -> Result<PathBuf> {
    let path = dir.join(name);
    std::fs::write(&path, contents)
        .with_context(|| format!("ファイルを書けません: {}", path.display()))?;
    Ok(path)
}
