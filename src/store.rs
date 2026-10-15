//! The files under `data/<topic>/`: the settings, the AOF, the head
//! sidecar, the heartbeat list, the payload blobs and the subscriptions.
//! File IO only; what the bytes mean is the writer's business, the heartbeat
//! scheduler's and the webhook deliverer's.

mod heartbeats;
mod payloads;
mod recover;
mod segment;
mod subscriptions;

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};

use crate::acceptor::GENESIS;

use crate::access::Grant;

pub use heartbeats::HeartbeatLog;
pub use payloads::{Payloads, Put};
pub use recover::{reconcile, Reconcile};
pub use segment::is_safe_segment;
pub use subscriptions::SubscriptionLog;

/// `topic.json`. Every member is required and nothing else is allowed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicSettings {
    pub topic_id: String,
    pub heartbeat_interval_seconds: u64,
    /// Where each heartbeat is POSTed once it is issued. May be empty.
    pub heartbeat_hosts: Vec<String>,
    /// The bearer tokens that may reach the topic's payloads, and as what. May be empty.
    pub access: Vec<Grant>,
}

/// The bytes of a `topic.json`, read against the schema and nothing more.
pub fn parse_settings(bytes: &[u8]) -> Result<TopicSettings, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("topic.json: {e}"))
}

/// Make `data/<id>/` and write `bytes` as its `topic.json`. The directory
/// is made exclusively, so an existing one is `AlreadyExists` and is left
/// as it was. `id` must already be a safe segment.
pub fn create_topic_dir(data: &Path, id: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let dir = data.join(id);
    fs::create_dir(&dir)?;
    let temporary = dir.join("topic.json.tmp");
    let mut file = File::create(&temporary)?;
    io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, dir.join("topic.json"))?;
    File::open(&dir)?.sync_all()?;
    File::open(data)?.sync_all()?;
    Ok(dir)
}

/// The AOF of the topic in `dir`. Readers open it by path; only the writer holds it open for writing.
pub fn aof_path(dir: &Path) -> PathBuf {
    dir.join("aof.jsonl")
}

/// The committed prefix: how many lines, the last line's hash, and the
/// prefix's length in bytes, which is where any uncommitted tail starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    pub count: u64,
    pub hash: [u8; 32],
    pub length: u64,
}

impl Head {
    pub const EMPTY: Head = Head { count: 0, hash: GENESIS, length: 0 };
}

/// The sidecar's bytes.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SidecarRecord {
    count: u64,
    head_hash: String,
    length: u64,
}

pub struct TopicDir {
    dir: PathBuf,
    aof: File,
}

impl TopicDir {
    /// Open the directory's files. The sidecar is absent for a topic that has never committed.
    pub fn open(dir: &Path) -> io::Result<(TopicDir, TopicSettings, Option<Head>)> {
        let settings = parse_settings(&fs::read(dir.join("topic.json"))?).map_err(io::Error::other)?;
        let aof = OpenOptions::new().create(true).write(true).read(true).open(aof_path(dir))?;
        let sidecar = match fs::read(dir.join("head")) {
            Ok(bytes) => Some(parse_sidecar(&bytes)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        Ok((TopicDir { dir: dir.to_path_buf(), aof }, settings, sidecar))
    }

    pub fn aof_length(&self) -> io::Result<u64> {
        Ok(self.aof.metadata()?.len())
    }

    /// Every byte from `from` to the end of the AOF.
    pub fn read_tail(&self, from: u64) -> io::Result<Vec<u8>> {
        let length = self.aof_length()?;
        let mut tail = vec![0u8; (length - from) as usize];
        self.aof.read_exact_at(&mut tail, from)?;
        Ok(tail)
    }

    /// `length` bytes from `offset`.
    pub fn read_at(&self, offset: u64, length: u64) -> io::Result<Vec<u8>> {
        let mut bytes = vec![0u8; length as usize];
        self.aof.read_exact_at(&mut bytes, offset)?;
        Ok(bytes)
    }

    /// The bytes from `from` to `to`, read as they are consumed rather than all at once.
    pub fn read_range(&self, from: u64, to: u64) -> io::Result<impl BufRead> {
        let mut file = self.aof.try_clone()?;
        file.seek(SeekFrom::Start(from))?;
        Ok(BufReader::new(file.take(to - from)))
    }

    /// Append the body and one terminator in one write, fsync, and return the new length.
    pub fn append(&mut self, body: &[u8], committed_length: u64) -> io::Result<u64> {
        let mut record = Vec::with_capacity(body.len() + 1);
        record.extend_from_slice(body);
        record.push(b'\n');
        self.aof.write_all_at(&record, committed_length)?;
        self.aof.sync_all()?;
        let expected = committed_length + record.len() as u64;
        let length = self.aof_length()?;
        if length != expected {
            return Err(io::Error::other(format!("AOF is {length} bytes after an append that should have made it {expected}")));
        }
        Ok(length)
    }

    /// Write the sidecar to a temporary file, fsync, rename over `head`, fsync the directory.
    pub fn write_head(&self, head: &Head) -> io::Result<()> {
        let record = SidecarRecord { count: head.count, head_hash: URL_SAFE_NO_PAD.encode(head.hash), length: head.length };
        let temporary = self.dir.join("head.tmp");
        let mut file = File::create(&temporary)?;
        io::Write::write_all(&mut file, &serde_json::to_vec(&record).expect("a record serialises"))?;
        file.sync_all()?;
        fs::rename(&temporary, self.dir.join("head"))?;
        File::open(&self.dir)?.sync_all()
    }
}

fn parse_sidecar(bytes: &[u8]) -> io::Result<Head> {
    let record: SidecarRecord = serde_json::from_slice(bytes).map_err(|e| io::Error::other(format!("head sidecar: {e}")))?;
    let decoded = URL_SAFE_NO_PAD.decode(&record.head_hash).map_err(|e| io::Error::other(format!("head sidecar hash: {e}")))?;
    let hash: [u8; 32] = decoded.try_into().map_err(|_| io::Error::other("head sidecar hash is not 32 bytes"))?;
    Ok(Head { count: record.count, hash, length: record.length })
}
