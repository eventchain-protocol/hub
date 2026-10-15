//! `subscriptions.jsonl`: the URLs a topic's receipts are POSTed to, one
//! JSON string per line, append-only. It is kept the way the heartbeat list
//! is: a URL is written and fsynced before it counts as subscribed, and a
//! last line without its terminator stops the Hub. Whether a URL is one the
//! Hub may POST to is the caller's rule; the file only refuses to hold the
//! same URL twice.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Mutex;

pub struct SubscriptionLog {
    file: File,
    committed: Mutex<Committed>,
}

struct Committed {
    length: u64,
    urls: Vec<String>,
}

impl SubscriptionLog {
    /// Open or create the file and load every URL in it.
    pub fn open(dir: &Path) -> io::Result<SubscriptionLog> {
        let file = OpenOptions::new().create(true).write(true).read(true).open(dir.join("subscriptions.jsonl"))?;
        let length = file.metadata()?.len();
        let mut bytes = vec![0u8; length as usize];
        file.read_exact_at(&mut bytes, 0)?;
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            return Err(io::Error::other("subscriptions.jsonl: the last line has no terminator"));
        }
        let mut urls: Vec<String> = Vec::new();
        for (number, line) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
            let url: String = serde_json::from_slice(&line[..line.len() - 1]).map_err(|e| io::Error::other(format!("subscriptions.jsonl line {}: {e}", number + 1)))?;
            if urls.contains(&url) {
                return Err(io::Error::other(format!("subscriptions.jsonl line {}: {url:?} is subscribed twice", number + 1)));
            }
            urls.push(url);
        }
        Ok(SubscriptionLog { file, committed: Mutex::new(Committed { length, urls }) })
    }

    /// Append `url` and fsync, unless it is already there. True if it was added.
    pub fn add(&self, url: &str) -> io::Result<bool> {
        let mut committed = self.committed.lock().expect("the subscription lock is not poisoned");
        if committed.urls.iter().any(|known| known == url) {
            return Ok(false);
        }
        let mut record = serde_json::to_vec(url).expect("a string serialises");
        record.push(b'\n');
        self.file.write_all_at(&record, committed.length)?;
        self.file.sync_all()?;
        committed.length += record.len() as u64;
        committed.urls.push(url.to_string());
        Ok(true)
    }

    pub fn all(&self) -> Vec<String> {
        self.committed.lock().expect("the subscription lock is not poisoned").urls.clone()
    }
}
