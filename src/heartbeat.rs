//! The heartbeat scheduler and publisher. Per topic, at the topic's
//! interval and whether or not anything arrived, the Hub signs a statement of
//! the committed head the writer last published, appends it to the topic's
//! heartbeat list, and POSTs it to each of the topic's heartbeat hosts.
//!
//! The statement has the members the specification appendix fixes, `v`,
//! `kind`, `topic_id`, `position`, `head_hash` and `issued_at`, and is signed
//! like a receipt. `position` is at least one, so a topic with no entries has
//! no heartbeat. The list is written before any host sees the statement, so
//! the Hub never shows a host a heartbeat it does not itself serve. A host
//! that does not take the POST is told nothing more: the failure goes to
//! stderr and is not retried, and the host can pull the list instead.

mod window;

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Url};
use tokio::time::MissedTickBehavior;

use crate::acceptor::Key;
use crate::signer::{base64url, issued_at_now, Statement};
use crate::topic::Topic;

pub use window::{Timestamp, Window};

/// Starts one task per topic, for the life of the process: the topics found
/// at startup and every topic created after.
pub struct Scheduler {
    key: Arc<Key>,
    client: Client,
}

impl Scheduler {
    pub fn new(key: Arc<Key>) -> Scheduler {
        Scheduler { key, client: Client::new() }
    }

    pub fn start(&self, topic: Arc<Topic>) {
        tokio::spawn(beat(Arc::clone(&self.key), topic, self.client.clone()));
    }
}

/// Tick at the interval, the first tick at once. A failure to sign or to
/// write the list stops this topic's heartbeats with the reason; the gap
/// shows at every host, as the protocol intends.
async fn beat(key: Arc<Key>, topic: Arc<Topic>, client: Client) {
    let interval = Duration::from_secs(topic.heartbeat_interval_seconds);
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let (key, issuer) = (Arc::clone(&key), Arc::clone(&topic));
        let issued = tokio::task::spawn_blocking(move || issue(&key, &issuer)).await.expect("issuing a heartbeat does not panic");
        let statement = match issued {
            Ok(Some(statement)) => statement,
            Ok(None) => continue,
            Err(e) => panic!("topic {}: heartbeats stopped: {e}", topic.id),
        };
        for host in &topic.heartbeat_hosts {
            tokio::spawn(publish(client.clone(), host.clone(), statement.clone(), topic.id.clone(), interval));
        }
    }
}

/// Sign the published head and append the statement to the list. Nothing while the topic has no entries.
fn issue(key: &Key, topic: &Topic) -> Result<Option<Vec<u8>>, String> {
    let head = topic.head();
    if head.count == 0 {
        return Ok(None);
    }
    let statement = Statement::new("heartbeat", &topic.id)
        .member("position", head.count.to_string())
        .member("head_hash", base64url(&head.hash))
        .member("issued_at", issued_at_now())
        .sign(key)
        .map_err(|e| format!("the Hub key did not sign: {e:?}"))?;
    topic.heartbeats.append(&statement).map_err(|e| format!("heartbeats.jsonl not written: {e}"))?;
    Ok(Some(statement))
}

/// One POST, given until the next heartbeat is due to answer.
async fn publish(client: Client, host: Url, statement: Vec<u8>, topic_id: String, timeout: Duration) {
    let sent = client.post(host.clone()).header(CONTENT_TYPE, "application/json").body(statement).timeout(timeout).send().await;
    if let Err(e) = sent.and_then(|response| response.error_for_status()) {
        eprintln!("topic {topic_id}: heartbeat not delivered to {host}: {e}");
    }
}

/// The `issued_at` of a statement from the list.
pub fn issued_at(statement: &[u8]) -> Result<Timestamp, String> {
    let members: std::collections::BTreeMap<String, String> = serde_json::from_slice(statement).map_err(|e| format!("a heartbeat in the list is not a statement: {e}"))?;
    let issued_at = members.get("issued_at").ok_or("a heartbeat in the list has no issued_at")?;
    Timestamp::parse(issued_at)
}
