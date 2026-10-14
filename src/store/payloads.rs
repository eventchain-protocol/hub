//! `payloads/<ref>`: the ciphertext blobs of a topic, one file per
//! `payload_ref`, holding exactly the bytes that were put. The Hub cannot
//! read them and does not try; integrity is the chain's, through the payload
//! hash in the Entry, so the store's only rule is that a blob, once put, is
//! never replaced.
//!
//! A put writes `payload.tmp` beside the directory, fsyncs it, and links it
//! in under the ref. A link, unlike a rename, refuses to replace an existing
//! name, so a blob that appeared by any other route is an error rather than
//! a silent overwrite. Puts within a topic are serialised by a lock, which is
//! what makes one temporary name enough.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Payloads {
    topic_dir: PathBuf,
    put_lock: Mutex<()>,
}

/// What a put did.
pub enum Put {
    /// The blob is new and durable.
    Created,
    /// The ref already held exactly these bytes; nothing was written.
    Identical,
    /// The ref already holds other bytes; nothing was written.
    Conflict,
}

impl Payloads {
    /// Open the topic's payload directory, creating it on first use.
    pub fn open(topic_dir: &Path) -> io::Result<Payloads> {
        fs::create_dir_all(topic_dir.join("payloads"))?;
        Ok(Payloads { topic_dir: topic_dir.to_path_buf(), put_lock: Mutex::new(()) })
    }

    /// The file of `reference`, which the caller has checked is a safe segment.
    pub fn path(&self, reference: &str) -> PathBuf {
        self.topic_dir.join("payloads").join(reference)
    }

    /// Store `bytes` under `reference` unless the ref is taken. Blocking IO.
    pub fn put(&self, reference: &str, bytes: &[u8]) -> io::Result<Put> {
        let _serialised = self.put_lock.lock().expect("the payload lock is not poisoned");
        let target = self.path(reference);
        match fs::read(&target) {
            Ok(existing) if existing == bytes => return Ok(Put::Identical),
            Ok(_) => return Ok(Put::Conflict),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let temporary = self.topic_dir.join("payload.tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::hard_link(&temporary, &target)?;
        fs::remove_file(&temporary)?;
        File::open(self.topic_dir.join("payloads"))?.sync_all()?;
        Ok(Put::Created)
    }
}
