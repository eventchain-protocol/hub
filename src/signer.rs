//! Signed statements. A heartbeat, a receipt and a refusal share one shape:
//! a flat JSON object of string members, signed by the Hub key over the JCS
//! serialisation of every member but `signature`.
//!
//! JCS for a string-only object with ASCII member names is members sorted by
//! name, no whitespace, strings escaped per RFC 8785 §3.2.2.2. That is what
//! serde_json writes from a sorted map, so the map is the canonicaliser.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

use crate::acceptor::{Error, Key};

pub struct Statement(BTreeMap<&'static str, String>);

impl Statement {
    /// `v`, `kind` and `topic_id` set; the caller adds the rest.
    pub fn new(kind: &'static str, topic_id: &str) -> Statement {
        let mut members = BTreeMap::new();
        members.insert("v", "1".to_string());
        members.insert("kind", kind.to_string());
        members.insert("topic_id", topic_id.to_string());
        Statement(members)
    }

    pub fn member(mut self, name: &'static str, value: impl Into<String>) -> Statement {
        self.0.insert(name, value.into());
        self
    }

    /// Sign the members as they stand, add `signature`, and serialise the whole.
    pub fn sign(mut self, key: &Key) -> Result<Vec<u8>, Error> {
        let message = serde_json::to_vec(&self.0).expect("a map of strings serialises");
        let signature = key.sign(&message)?;
        self.0.insert("signature", base64url(&signature));
        Ok(serde_json::to_vec(&self.0).expect("a map of strings serialises"))
    }
}

/// The entry specification's binary encoding: base64url without padding.
pub fn base64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Unix seconds, with a fractional part to the millisecond and no trailing zeros.
pub fn issued_at_now() -> String {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).expect("the clock is after 1970");
    let millis = since_epoch.subsec_millis();
    if millis == 0 {
        since_epoch.as_secs().to_string()
    } else {
        let fraction = format!("{millis:03}");
        format!("{}.{}", since_epoch.as_secs(), fraction.trim_end_matches('0'))
    }
}
