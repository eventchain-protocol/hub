//! Configuration and wiring. Everything else lives in the module named for
//! the concept it carries.

mod acceptor;
mod api;
mod heartbeat;
mod index;
mod signer;
mod store;
mod topic;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn key_from_env() -> acceptor::Key {
    let hex = std::env::var("HUB_KEY").expect("HUB_KEY must be set to 64 hex characters");
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("HUB_KEY must be hex"))
        .collect();
    let bytes: [u8; 32] = bytes.try_into().expect("HUB_KEY must be 32 bytes");
    acceptor::Key::load(bytes).unwrap_or_else(|e| panic!("HUB_KEY is not a P-256 private scalar: {e:?}"))
}

/// Every directory under the data directory with a `topic.json` is a topic. Any of them failing to open stops the Hub.
fn topics_from(data: &Path, index: &Arc<index::Index>) -> HashMap<String, Arc<topic::Topic>> {
    let entries = fs::read_dir(data).unwrap_or_else(|e| panic!("data directory {}: {e}", data.display()));
    let mut topics = HashMap::new();
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if !path.join("topic.json").is_file() {
            continue;
        }
        let topic = topic::Topic::open(&path, Arc::clone(index)).unwrap_or_else(|e| panic!("{e}"));
        topics.insert(topic.id.clone(), Arc::new(topic));
    }
    if topics.is_empty() {
        panic!("no topic directories under {}", data.display());
    }
    topics
}

#[tokio::main]
async fn main() {
    let _runtime = acceptor::Runtime::start();
    let key = Arc::new(key_from_env());
    let data = std::env::var("HUB_DATA").unwrap_or_else(|_| "data".to_string());
    println!("hub key {}", signer::base64url(&key.public_key()));
    let index = Arc::new(index::Index::open(&Path::new(&data).join("index")).unwrap_or_else(|e| panic!("{e}")));
    let topics = topics_from(Path::new(&data), &index);
    for (id, topic) in &topics {
        let head = topic.head();
        println!("topic {id}: {} entries, heartbeat every {} s to {} hosts", head.count, topic.heartbeat_interval_seconds, topic.heartbeat_hosts.len());
    }
    heartbeat::start(Arc::clone(&key), topics.values().cloned().collect());
    let router = api::router(Arc::new(api::App { key, topics }));
    let addr = std::env::var("HUB_ADDR").unwrap_or_else(|_| "127.0.0.1:3717".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| panic!("bind {addr}: {e}"));
    println!("listening on http://{addr}");
    axum::serve(listener, router).await.expect("serve");
}
