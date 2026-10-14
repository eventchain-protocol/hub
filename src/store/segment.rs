//! The naming rule for anything a request turns into a file or directory
//! name under the data directory: a topic id and a payload ref. Both are
//! opaque to the Hub, so the rule says nothing about their meaning, only
//! that each is exactly one path segment that cannot climb out of the
//! directory it is joined to. A pure decision, so it is checked without a file.

/// Non-empty, ASCII printable throughout, no `/`, no `..` anywhere, and not `.`.
pub fn is_safe_segment(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| (0x20..=0x7e).contains(&b))
        && !name.contains('/')
        && !name.contains("..")
        && name != "."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_segments() {
        let cases: [(&str, bool); 14] = [
            ("demo", true),
            ("x2YbRvM-_q0", true),
            ("a.b", true),
            ("with space", true),
            ("", false),
            (".", false),
            ("..", false),
            ("a..b", false),
            ("../etc", false),
            ("a/b", false),
            ("/", false),
            ("tab\there", false),
            ("caf\u{e9}", false),
            ("del\u{7f}", false),
        ];
        for (name, want) in cases {
            assert_eq!(is_safe_segment(name), want, "{name:?}");
        }
    }
}
