//! JavaScript string semantics the vault format depends on (trim, \s, UTF-16 lengths/indexes).

/// ECMAScript WhiteSpace + LineTerminator (what `String.prototype.trim` and `/\s/` match).
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{0009}' | '\u{000A}' | '\u{000B}' | '\u{000C}' | '\u{000D}' | '\u{0020}' | '\u{00A0}' | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `String(s).trim().toLowerCase()`
pub fn normalize_answer(s: &str) -> String {
    js_trim(s).to_lowercase()
}

/// `s.length` in JavaScript (UTF-16 code units).
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s[i]` in JavaScript: the UTF-16 code unit at `i` as a string (a lone surrogate becomes U+FFFD).
pub fn utf16_at(s: &str, i: usize) -> Option<String> {
    let unit = s.encode_utf16().nth(i)?;
    Some(String::from_utf16_lossy(&[unit]))
}

/// Indexes (UTF-16) of every ' ' in `s`, as answer-questions.js records them.
pub fn space_indexes(s: &str) -> Vec<u32> {
    s.encode_utf16().enumerate().filter(|(_, u)| *u == 0x20).map(|(i, _)| i as u32).collect()
}

/// `s.replace(/\s+/g, '')`
pub fn strip_js_space(s: &str) -> String {
    s.chars().filter(|c| !is_js_space(*c)).collect()
}

/// `s.slice(0, n)` by UTF-16 units (a split surrogate pair is dropped).
pub fn utf16_slice(s: &str, n: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().take(n).collect();
    let mut out = String::from_utf16_lossy(&units);
    if out.ends_with('\u{FFFD}') && !s.encode_utf16().take(n).eq(out.encode_utf16()) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trims_like_js() {
        assert_eq!(js_trim("\u{FEFF}\u{00A0} a b \n"), "a b");
        assert_eq!(js_trim("\u{0085}x"), "\u{0085}x"); // NEL is not JS whitespace
        assert_eq!(normalize_answer("  Tomasz WILEY "), "tomasz wiley");
    }
    #[test]
    fn utf16() {
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(space_indexes("ruth ann x"), vec![4, 8]);
        assert_eq!(utf16_at("abc", 1).as_deref(), Some("b"));
        assert_eq!(utf16_slice("ab😀", 3), "ab");
    }
}
