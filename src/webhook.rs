//! Webhook delivery: after each accepted submission, the receipt, the exact
//! signed bytes the Producer got, is POSTed to every URL subscribed to the
//! topic. The paper has systems subscribe to a topic over HTTP and receive
//! each entry as the Hub appends it; the receipt names the new head and the
//! subscriber fetches the lines it does not have, then verifies them.
//!
//! Each delivery is its own task: one attempt at once, then a retry after
//! 1, 4, 16 and 64 seconds, each attempt given 10 seconds. A 2xx is
//! delivered; any other status, a redirect included, a timeout or a
//! connection error is a failed attempt, logged to stderr, and after the
//! last the delivery is given up. Deliveries live in memory only. A restart
//! drops what was not yet delivered, and deliveries to one subscriber may
//! arrive out of order. A subscriber reconciles through
//! `GET /topics/{t}/entries?after=`, which is what makes in-memory delivery
//! enough: the file is the record, a webhook only a prompt to read it.
//!
//! The subscriptions themselves are the topic's (`subscriptions.jsonl`);
//! this module only reads them when a receipt is handed to it.

use std::time::Duration;

use bytes::Bytes;
use reqwest::header::CONTENT_TYPE;
use reqwest::{redirect, Client};

use crate::topic::Topic;

/// The waits before each retry. Their count is the number of retries.
const RETRY_AFTER_SECONDS: [u64; 4] = [1, 4, 16, 64];

/// How long one attempt may take.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Deliveries {
    client: Client,
}

impl Deliveries {
    pub fn new() -> Deliveries {
        let client = Client::builder().redirect(redirect::Policy::none()).timeout(ATTEMPT_TIMEOUT).build().expect("an HTTP client with no redirects builds");
        Deliveries { client }
    }

    /// Send the receipt for the submission that made `position` to every current subscriber of `topic`.
    pub fn receipt(&self, topic: &Topic, position: u64, receipt: Bytes) {
        for url in topic.subscriptions.all() {
            tokio::spawn(deliver(self.client.clone(), topic.id.clone(), position, url, receipt.clone()));
        }
    }
}

async fn deliver(client: Client, topic_id: String, position: u64, url: String, receipt: Bytes) {
    let mut retries = RETRY_AFTER_SECONDS.iter();
    for attempt in 1.. {
        let sent = client.post(&url).header(CONTENT_TYPE, "application/json").body(receipt.clone()).send().await;
        let failure = match sent.and_then(|response| response.error_for_status()) {
            Ok(response) if response.status().is_success() => return,
            Ok(response) => format!("HTTP {}", response.status()),
            Err(e) => e.to_string(),
        };
        match retries.next() {
            Some(seconds) => {
                eprintln!("topic {topic_id}: receipt at position {position} not delivered to {url}, attempt {attempt}: {failure}; retrying in {seconds} s");
                tokio::time::sleep(Duration::from_secs(*seconds)).await;
            }
            None => {
                eprintln!("topic {topic_id}: receipt at position {position} not delivered to {url}, attempt {attempt}: {failure}; giving up");
                return;
            }
        }
    }
}
