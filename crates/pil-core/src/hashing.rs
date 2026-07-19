//! 決定論的ハッシュのための共通フレーミング．
//!
//! 連結の曖昧さ（`("ab","c")` と `("a","bc")` が衝突する）を避けるため，
//! 各フィールドを «長さ（u64 LE）‖ 本体» でフレーミングし，先頭にドメインタグを置く．
//! ドメインタグにより `CaseId` と `ContentKey` のハッシュ空間を分離する．

/// ドメイン分離つきのフレーム化 blake3 ハッシュ．
///
/// `fields` は順序を保った可変長バイト列の並び．`Option` の有無を表現したい場合は
/// 呼び出し側が presence フラグ（`b"\x00"` / `b"\x01"`）を別フィールドとして与える．
pub(crate) fn framed_hash(domain: &str, fields: &[&[u8]]) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    h.update(&(domain.len() as u64).to_le_bytes());
    h.update(domain.as_bytes());
    for f in fields {
        h.update(&(f.len() as u64).to_le_bytes());
        h.update(f);
    }
    h.finalize()
}

/// `Option<&str>` を «presence フラグ, 本体» の 2 フィールドに展開して push するヘルパ．
/// `None` と `Some("")` を確実に区別する．
pub(crate) fn push_opt_str<'a>(fields: &mut Vec<&'a [u8]>, value: Option<&'a str>) {
    match value {
        None => fields.push(b"\x00"),
        Some(s) => {
            fields.push(b"\x01");
            fields.push(s.as_bytes());
        }
    }
}
