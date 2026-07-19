//! Ollama バージョン要件の判定（DESIGN §6.3 / §11.1）．
//!
//! Ollama は **v0.12.11** で `logprobs` / `top_logprobs` に対応した（§6.3）．`pil-llm` の
//! Ollama バックエンドは `>= 0.12.11` を要求し，起動時に検査する（§8.3 の judge が黙って
//! 劣化しないため）．ここは reqwest 非依存の純関数で，既定ビルドでも単体テストできる．

/// 要求する Ollama 最小バージョン（DESIGN §6.3）．
pub const OLLAMA_MIN_VERSION: (u32, u32, u32) = (0, 12, 11);

/// `OLLAMA_MIN_VERSION` を `"major.minor.patch"` 文字列で返す（エラー表示用）．
pub fn min_version_string() -> String {
    let (a, b, c) = OLLAMA_MIN_VERSION;
    format!("{a}.{b}.{c}")
}

/// `"0.12.11"` / `"v0.12.11"` / `"0.12.11-rc1"` 等を `(major, minor, patch)` に解釈する．
///
/// 先頭の `v` と，`-` / `+` 以降の pre-release / build メタデータは無視する．
/// minor / patch が省略されている場合は 0 とみなす．
pub fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let core = s.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let mut it = core.split('.');
    let major = it.next()?.parse::<u32>().ok()?;
    let minor = it.next().unwrap_or("0").parse::<u32>().ok()?;
    let patch = it.next().unwrap_or("0").parse::<u32>().ok()?;
    Some((major, minor, patch))
}

/// `found` が `min` 以上か（解釈不能なら false = 要件未達扱い）．
pub fn meets_minimum(found: &str, min: (u32, u32, u32)) -> bool {
    match parse_semver(found) {
        // タプル比較は (major, minor, patch) の辞書式順序になる
        Some(v) => v >= min,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variants() {
        assert_eq!(parse_semver("0.12.11"), Some((0, 12, 11)));
        assert_eq!(parse_semver("v0.12.11"), Some((0, 12, 11)));
        assert_eq!(parse_semver("0.12.11-rc1"), Some((0, 12, 11)));
        assert_eq!(parse_semver("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_semver("garbage"), None);
    }

    #[test]
    fn enforces_minimum() {
        // 満たす
        assert!(meets_minimum("0.12.11", OLLAMA_MIN_VERSION));
        assert!(meets_minimum("0.12.12", OLLAMA_MIN_VERSION));
        assert!(meets_minimum("0.13.0", OLLAMA_MIN_VERSION));
        assert!(meets_minimum("1.0.0", OLLAMA_MIN_VERSION));
        // 満たさない
        assert!(!meets_minimum("0.12.10", OLLAMA_MIN_VERSION));
        assert!(!meets_minimum("0.11.99", OLLAMA_MIN_VERSION));
        assert!(!meets_minimum("0.9.0", OLLAMA_MIN_VERSION));
        // 解釈不能は未達扱い
        assert!(!meets_minimum("unknown", OLLAMA_MIN_VERSION));
    }

    #[test]
    fn min_version_string_matches_const() {
        assert_eq!(min_version_string(), "0.12.11");
    }
}
