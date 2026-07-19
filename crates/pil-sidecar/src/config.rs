//! sidecar の構成（[`SidecarConfig`]）と実行 provenance（[`SidecarRun`]）の純データ型（DESIGN §4.1）．
//!
//! これらの型は **feature 非依存**であり，`sidecar` feature 無し（既定 network-free）でも参照・単体
//! テストできる．env 注入マップの組み立てや provenance の整形は副作用を持たない純ロジックであり，
//! 実際のプロセス起動（[`crate::launcher`]）から切り離してテストできる（§4.1 の「グルーは Rust に集約」）．

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Python の OpenAI 互換クライアントが読む base_url の環境変数名．
pub const ENV_OPENAI_BASE_URL: &str = "OPENAI_BASE_URL";

/// Python の OpenAI 互換クライアントが読む API 鍵の環境変数名．
pub const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";

/// シムはローカルのため実鍵は不要．OpenAI 型クライアントが空鍵で失敗しないよう置くダミー値．
pub const DUMMY_API_KEY: &str = "pil-shim";

/// Python sidecar の起動方法を記述する構成（DESIGN §4.1 native-first）．
///
/// 「何を・どこで起動し，どのシムへ向けるか」だけを持つ．モデル呼び出しの温度・seed・cache・
/// metadata は全てシム経由で pil-llm 側に揃うため，ここには一切持たない（制御の反転）．
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarConfig {
    /// 起動プログラム（例 `"python3"`）．
    pub program: String,
    /// 実行するスクリプトのパス（薄い殻の Python 本体）．
    pub script: PathBuf,
    /// スクリプトへ渡す追加引数．
    pub args: Vec<String>,
    /// 作業ディレクトリ（省略時は親プロセスを継承）．
    pub working_dir: Option<PathBuf>,
    /// 注入するシムの base_url（例 `"http://127.0.0.1:<port>/v1"`）．
    pub base_url: String,
    /// 注入する API 鍵．シムはローカルのため既定は [`DUMMY_API_KEY`]．
    pub api_key: String,
}

impl SidecarConfig {
    /// `program` でスクリプト `script` を起動し，モデル呼び出しを `base_url` のシムへ向ける構成を作る．
    ///
    /// `args` は空・`working_dir` は継承・`api_key` は [`DUMMY_API_KEY`] を既定とする．
    pub fn new(
        program: impl Into<String>,
        script: impl Into<PathBuf>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            program: program.into(),
            script: script.into(),
            args: Vec::new(),
            working_dir: None,
            base_url: base_url.into(),
            api_key: DUMMY_API_KEY.to_string(),
        }
    }

    /// スクリプト引数を差し替える（builder）．
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// 作業ディレクトリを設定する（builder）．
    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// シムへ向けるために注入する環境変数マップを組み立てる（§4.1）．
    ///
    /// `OPENAI_BASE_URL`＝シム base_url，`OPENAI_API_KEY`＝ダミー鍵の 2 件のみ．これにより Python の
    /// OpenAI 互換クライアントは第 2 のモデル経路を開かず，pil-llm 単一経路へ routing される．
    /// 決定論のため `BTreeMap`（キー順安定）で返す．
    pub fn injected_env(&self) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert(ENV_OPENAI_BASE_URL.to_string(), self.base_url.clone());
        env.insert(ENV_OPENAI_API_KEY.to_string(), self.api_key.clone());
        env
    }
}

/// sidecar 1 回の実行の provenance 記録（DESIGN §4.1）．
///
/// 「何を・どの引数で起動し，どのシムへ向け，どう終わったか」を正規化して残す．**wall-clock の
/// 経過時間は含めない**（測定の再現性を壊さないため；実時間が要る箇所は呼び出し側で別途扱う）．
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarRun {
    /// 起動したプログラム．
    pub program: String,
    /// 渡した引数列．
    pub args: Vec<String>,
    /// 注入したシムの base_url（§4.1 の funnel 先）．
    pub base_url: String,
    /// 正常終了（exit status success）か．
    pub success: bool,
    /// 終了コード（シグナル終了時は `None`）．
    pub exit_code: Option<i32>,
    /// 捕捉した標準出力（UTF-8 に正規化）．
    pub stdout: String,
    /// 捕捉した標準エラー（UTF-8 に正規化）．
    pub stderr: String,
}

impl SidecarRun {
    /// 構成と捕捉結果から provenance 記録を整形する（純ロジック；プロセス起動に依存しない）．
    pub fn new(
        config: &SidecarConfig,
        success: bool,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    ) -> Self {
        Self {
            program: config.program.clone(),
            args: config.args.clone(),
            base_url: config.base_url.clone(),
            success,
            exit_code,
            stdout,
            stderr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_defaults() {
        let config = SidecarConfig::new("python3", "stub.py", "http://127.0.0.1:8080/v1");
        assert_eq!(config.program, "python3");
        assert_eq!(config.script, PathBuf::from("stub.py"));
        assert!(config.args.is_empty());
        assert_eq!(config.working_dir, None);
        assert_eq!(config.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(config.api_key, DUMMY_API_KEY);
    }

    #[test]
    fn builders_set_args_and_working_dir() {
        let config = SidecarConfig::new("python3", "stub.py", "http://127.0.0.1:1/v1")
            .with_args(["--flag", "value"])
            .with_working_dir("/work");
        assert_eq!(config.args, vec!["--flag".to_string(), "value".to_string()]);
        assert_eq!(config.working_dir, Some(PathBuf::from("/work")));
    }

    #[test]
    fn injected_env_routes_to_shim() {
        let config = SidecarConfig::new("python3", "stub.py", "http://127.0.0.1:9/v1");
        let env = config.injected_env();
        assert_eq!(env.len(), 2);
        assert_eq!(
            env.get(ENV_OPENAI_BASE_URL).map(String::as_str),
            Some("http://127.0.0.1:9/v1")
        );
        assert_eq!(
            env.get(ENV_OPENAI_API_KEY).map(String::as_str),
            Some(DUMMY_API_KEY)
        );
    }

    #[test]
    fn injected_env_reflects_custom_api_key() {
        let mut config = SidecarConfig::new("python3", "stub.py", "http://127.0.0.1:9/v1");
        config.api_key = "custom-key".to_string();
        let env = config.injected_env();
        assert_eq!(
            env.get(ENV_OPENAI_API_KEY).map(String::as_str),
            Some("custom-key")
        );
    }

    #[test]
    fn run_shaping_copies_config_provenance() {
        let config =
            SidecarConfig::new("python3", "stub.py", "http://127.0.0.1:5/v1").with_args(["a", "b"]);
        let run = SidecarRun::new(
            &config,
            true,
            Some(0),
            "MOCK response ...".to_string(),
            String::new(),
        );
        assert_eq!(run.program, "python3");
        assert_eq!(run.args, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(run.base_url, "http://127.0.0.1:5/v1");
        assert!(run.success);
        assert_eq!(run.exit_code, Some(0));
        assert!(run.stdout.contains("MOCK response"));
        assert!(run.stderr.is_empty());
    }

    #[test]
    fn run_records_failure_without_exit_code() {
        let config = SidecarConfig::new("python3", "stub.py", "http://127.0.0.1:5/v1");
        let run = SidecarRun::new(&config, false, None, String::new(), "boom".to_string());
        assert!(!run.success);
        assert_eq!(run.exit_code, None);
        assert_eq!(run.stderr, "boom");
    }
}
