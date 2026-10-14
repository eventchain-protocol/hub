//! `heartbeats.jsonl`: every heartbeat the Hub has issued for the topic, one
//! signed statement per line, append-only. The Hub serves its list endpoints
//! from this file, so it is kept the way the AOF is: a statement is written
//! and fsynced before it is published to readers or posted to a host, and a
//! reader sees only the committed prefix, never a line still being written.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Mutex;

pub struct HeartbeatLog {
    file: File,
    committed: Mutex<Committed>,
}

/// The committed prefix of the file, and its last statement for `heartbeat/latest`.
struct Committed {
    length: u64,
    latest: Option<Vec<u8>>,
}

/// The statements in `bytes`, each without its terminator. `bytes` is a committed prefix, so every line is whole.
fn statements(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split_inclusive(|b| *b == b'\n').map(|line| &line[..line.len() - 1])
}

impl HeartbeatLog {
    /// Open or create the file. A last line without its terminator is a
    /// heartbeat cut off mid-write; it stops the Hub as a torn AOF does.
    pub fn open(dir: &Path) -> io::Result<HeartbeatLog> {
        let file = OpenOptions::new().create(true).write(true).read(true).open(dir.join("heartbeats.jsonl"))?;
        let length = file.metadata()?.len();
        let mut bytes = vec![0u8; length as usize];
        file.read_exact_at(&mut bytes, 0)?;
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            return Err(io::Error::other("heartbeats.jsonl: the last line has no terminator"));
        }
        let latest = statements(&bytes).last().map(<[u8]>::to_vec);
        Ok(HeartbeatLog { file, committed: Mutex::new(Committed { length, latest }) })
    }

    /// Append one statement and its terminator, fsync, then publish it.
    /// Holding the lock across the write keeps the file single-writer.
    pub fn append(&self, statement: &[u8]) -> io::Result<()> {
        let mut committed = self.committed.lock().expect("the heartbeat lock is not poisoned");
        let mut record = Vec::with_capacity(statement.len() + 1);
        record.extend_from_slice(statement);
        record.push(b'\n');
        self.file.write_all_at(&record, committed.length)?;
        self.file.sync_all()?;
        committed.length += record.len() as u64;
        committed.latest = Some(statement.to_vec());
        Ok(())
    }

    pub fn latest(&self) -> Option<Vec<u8>> {
        self.committed.lock().expect("the heartbeat lock is not poisoned").latest.clone()
    }

    /// Every committed statement, oldest first, each without its terminator.
    pub fn all(&self) -> io::Result<Vec<Vec<u8>>> {
        let length = self.committed.lock().expect("the heartbeat lock is not poisoned").length;
        let mut bytes = vec![0u8; length as usize];
        self.file.read_exact_at(&mut bytes, 0)?;
        Ok(statements(&bytes).map(<[u8]>::to_vec).collect())
    }
}
