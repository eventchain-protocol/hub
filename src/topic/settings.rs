//! A topic's settings as the Hub runs them: `topic.json` read against its
//! schema, then held to the rules the schema cannot state. The same rules
//! apply to a topic found on disk at startup and to one created over HTTP,
//! so a topic that would not open is never created.

use reqwest::Url;

use crate::access::TopicTokens;
use crate::store::{is_safe_segment, TopicSettings};

pub struct Settings {
    pub id: String,
    pub heartbeat_interval_seconds: u64,
    pub heartbeat_hosts: Vec<Url>,
    pub access: TopicTokens,
}

impl Settings {
    pub fn check(settings: TopicSettings) -> Result<Settings, String> {
        let id = settings.topic_id;
        if !is_safe_segment(&id) || id == "index" {
            return Err(format!("topic.json: topic_id {id:?} is not a single safe path segment other than \"index\""));
        }
        if settings.heartbeat_interval_seconds == 0 {
            return Err(format!("{id}: topic.json: heartbeat_interval_seconds must be at least 1"));
        }
        let heartbeat_hosts = settings
            .heartbeat_hosts
            .iter()
            .map(|host| http_url(host).map_err(|e| format!("{id}: topic.json: heartbeat host: {e}")))
            .collect::<Result<Vec<Url>, String>>()?;
        let access = TopicTokens::new(&settings.access).map_err(|e| format!("{id}: topic.json: {e}"))?;
        Ok(Settings { id, heartbeat_interval_seconds: settings.heartbeat_interval_seconds, heartbeat_hosts, access })
    }
}

/// A URL the Hub may POST to: absolute, http or https, with nothing around it.
pub fn http_url(text: &str) -> Result<Url, String> {
    if text.trim() != text {
        return Err(format!("{text:?} has whitespace around it"));
    }
    let url = Url::parse(text).map_err(|e| format!("{text:?}: {e}"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(format!("{text:?} is not http or https"));
    }
    Ok(url)
}
