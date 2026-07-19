//! 依存を増やさず `YYYYMMDD_HHMMSS`（UTC）を作る（DESIGN §11.3 の成果物タイムスタンプ）．
//!
//! `results/{subcommand}_YYYYMMDD_HHMMSS/` のディレクトリ名に使う．外部 crate（chrono/time）を
//! 足さずに `std::time` だけで賄い，既定ビルドのネットワーク非依存・ビルド再現性を保つ．
//! 暦計算は Howard Hinnant の `civil_from_days`（proleptic Gregorian）を用いる．

use std::time::{SystemTime, UNIX_EPOCH};

/// 現在時刻（UTC）を `YYYYMMDD_HHMMSS` 文字列にする．
pub fn now_utc_compact() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_compact(secs)
}

/// Unix 秒（UTC）を `YYYYMMDD_HHMMSS` にする（テスト可能な純関数）．
pub fn format_compact(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}{mo:02}{d:02}_{hh:02}{mm:02}{ss:02}")
}

/// エポック（1970-01-01）からの経過日数を `(year, month, day)`（UTC）に変換する．
///
/// Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms" の `civil_from_days`．
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_epochs_format_correctly() {
        // 1970-01-01 00:00:00 UTC
        assert_eq!(format_compact(0), "19700101_000000");
        // 2021-01-01 00:00:00 UTC = 1609459200
        assert_eq!(format_compact(1_609_459_200), "20210101_000000");
        // 2025-01-01 00:00:00 UTC = 1735689600
        assert_eq!(format_compact(1_735_689_600), "20250101_000000");
        // 2025-01-01 12:34:56 UTC = 1735689600 + 45296
        assert_eq!(format_compact(1_735_734_896), "20250101_123456");
    }

    #[test]
    fn shape_is_stable() {
        let s = now_utc_compact();
        assert_eq!(s.len(), 15);
        assert_eq!(&s[8..9], "_");
    }
}
