//! The `from`/`until` window of the heartbeat range query. The appendix
//! selects the heartbeats with `from <= issued_at < until`, an omitted bound
//! open, and writes every timestamp as Unix seconds in ASCII decimal digits
//! without leading zeros, optionally followed by a point and fractional
//! digits without trailing zeros. Compared as text those would sort wrongly
//! (`"9" > "10"`); compared as floats they would lose digits. A timestamp
//! here is the whole seconds as an integer and the fractional digits as
//! text, which compare correctly in that order: with no trailing zeros, the
//! fraction digits' byte order is their numeric order.

/// A timestamp in the appendix's form.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    seconds: u64,
    fraction: String,
}

impl Timestamp {
    /// Parse exactly the appendix's form; anything else is an error.
    pub fn parse(text: &str) -> Result<Timestamp, String> {
        let (whole, fraction) = match text.split_once('.') {
            Some((whole, fraction)) => (whole, fraction),
            None => (text, ""),
        };
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(whole) || (whole.len() > 1 && whole.starts_with('0')) {
            return Err(format!("{text:?} is not Unix seconds in decimal digits without leading zeros"));
        }
        if text.contains('.') && (!digits(fraction) || fraction.ends_with('0')) {
            return Err(format!("{text:?} has a fractional part that is not decimal digits without trailing zeros"));
        }
        let seconds = whole.parse().map_err(|_| format!("{text:?} is too large"))?;
        Ok(Timestamp { seconds, fraction: fraction.to_string() })
    }
}

/// The selected range; `None` is an open end.
pub struct Window {
    pub from: Option<Timestamp>,
    pub until: Option<Timestamp>,
}

impl Window {
    pub fn contains(&self, issued_at: &Timestamp) -> bool {
        self.from.as_ref().is_none_or(|from| from <= issued_at) && self.until.as_ref().is_none_or(|until| issued_at < until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_the_appendix_form() {
        let cases: [(&str, bool); 16] = [
            ("0", true),
            ("1760000000", true),
            ("1760000000.5", true),
            ("1760000000.05", true),
            ("0.001", true),
            ("", false),
            ("01", false),
            ("1.", false),
            (".5", false),
            ("1.50", false),
            ("1.0", false),
            ("+1", false),
            ("-1", false),
            ("1e3", false),
            ("1.5.2", false),
            ("99999999999999999999", false),
        ];
        for (text, valid) in cases {
            assert_eq!(Timestamp::parse(text).is_ok(), valid, "{text:?}");
        }
    }

    #[test]
    fn selects_from_inclusive_until_exclusive() {
        let cases: [(&str, Option<&str>, Option<&str>, bool); 12] = [
            ("10", None, None, true),
            ("10", Some("10"), None, true),
            ("10", Some("10.001"), None, false),
            ("10", None, Some("10"), false),
            ("9.999", None, Some("10"), true),
            ("100", Some("20"), None, true),
            ("9", Some("10"), Some("11"), false),
            ("10.5", Some("10.25"), Some("10.75"), true),
            ("10.75", Some("10.25"), Some("10.75"), false),
            ("10.25", Some("10.25"), Some("10.75"), true),
            ("10.1", Some("10.05"), None, true),
            ("10.05", Some("10.1"), None, false),
        ];
        for (issued_at, from, until, selected) in cases {
            let window = Window { from: from.map(|t| Timestamp::parse(t).unwrap()), until: until.map(|t| Timestamp::parse(t).unwrap()) };
            assert_eq!(window.contains(&Timestamp::parse(issued_at).unwrap()), selected, "{issued_at} in [{from:?}, {until:?})");
        }
    }
}
