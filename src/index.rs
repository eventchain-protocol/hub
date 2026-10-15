//! The Event ID index: for every committed line, where it sits in its
//! topic's AOF. One embedded redb store under `data/index/` serves every
//! topic, so a lookup costs a B-tree read however long the file is, and the
//! store survives a restart instead of being rebuilt in memory.
//!
//! The index is derived from the files and is never authoritative. The
//! writer extends it after the head sidecar is replaced, and on open a topic
//! catches it up from the AOF, hashing each line the way the acceptor does.
//! An index behind the sidecar is the normal result of a crash or a deleted
//! directory; one ahead of it, or naming a different history, stops the Hub.
//!
//! An Entry carries no topic, so the same line can be committed in two
//! topics. The key is therefore the pair (topic, Event ID), not the Event ID
//! alone, and a lookup is always within one topic.

use std::fs;
use std::io::BufRead;
use std::path::Path;

use redb::{Database, ReadableTable, TableDefinition};
use sha2::{Digest, Sha256};

use crate::signer::base64url;
use crate::store::{Head, TopicDir};

/// (topic, Event ID) → (position, byte offset of the line, line length without its terminator).
const LINES: TableDefinition<(&str, [u8; 32]), (u64, u64, u64)> = TableDefinition::new("lines");

/// topic → (count, byte length, last line's hash) of the prefix the index covers.
const PREFIX: TableDefinition<&str, (u64, u64, [u8; 32])> = TableDefinition::new("prefix");

/// How many lines one write transaction indexes, so that rebuilding a long
/// file holds a bounded transaction and a rebuild cut short resumes where it stopped.
const LINES_PER_TRANSACTION: u64 = 10_000;

pub struct Index {
    db: Database,
}

/// Where one line sits: its position (the count of the prefix it ends), and its bytes' place in the file.
#[derive(Clone, Copy, Debug)]
pub struct Located {
    pub position: u64,
    pub offset: u64,
    pub length: u64,
}

impl Located {
    /// The byte offset just past the line's terminator, where the extension after it starts.
    pub fn end(&self) -> u64 {
        self.offset + self.length + 1
    }
}

fn store_error(e: impl Into<redb::Error>) -> String {
    format!("index: {}", e.into())
}

fn event_id(line: &[u8]) -> [u8; 32] {
    Sha256::digest(line).into()
}

impl Index {
    /// Open or create the store in `dir`, which must not be a topic.
    pub fn open(dir: &Path) -> Result<Index, String> {
        if dir.join("topic.json").exists() {
            return Err(format!("{} is the Event ID index and cannot be a topic", dir.display()));
        }
        fs::create_dir_all(dir).map_err(|e| format!("index directory {}: {e}", dir.display()))?;
        let db = Database::create(dir.join("events.redb")).map_err(store_error)?;
        let transaction = db.begin_write().map_err(store_error)?;
        transaction.open_table(LINES).map_err(store_error)?;
        transaction.open_table(PREFIX).map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(Index { db })
    }

    /// The prefix of `topic` the index covers; empty for a topic it has never seen.
    pub fn indexed(&self, topic: &str) -> Result<Head, String> {
        let transaction = self.db.begin_read().map_err(store_error)?;
        let table = transaction.open_table(PREFIX).map_err(store_error)?;
        Ok(match table.get(topic).map_err(store_error)? {
            Some(record) => {
                let (count, length, hash) = record.value();
                Head { count, hash, length }
            }
            None => Head::EMPTY,
        })
    }

    pub fn locate(&self, topic: &str, event_id: &[u8; 32]) -> Result<Option<Located>, String> {
        let transaction = self.db.begin_read().map_err(store_error)?;
        let table = transaction.open_table(LINES).map_err(store_error)?;
        Ok(table.get((topic, *event_id)).map_err(store_error)?.map(|record| {
            let (position, offset, length) = record.value();
            Located { position, offset, length }
        }))
    }

    /// Index every line `lines` yields, each ending in `0x0a`, as the lines
    /// that follow the prefix already indexed, and require the result to be
    /// exactly `to`. The writer calls this with the submission it has just
    /// committed; the catch-up calls it with the file beyond the indexed prefix.
    pub fn extend(&self, topic: &str, lines: &mut impl BufRead, to: &Head) -> Result<(), String> {
        loop {
            let transaction = self.db.begin_write().map_err(store_error)?;
            let (prefix, finished) = {
                let mut prefix_table = transaction.open_table(PREFIX).map_err(store_error)?;
                let mut lines_table = transaction.open_table(LINES).map_err(store_error)?;
                let mut prefix = match prefix_table.get(topic).map_err(store_error)? {
                    Some(record) => {
                        let (count, length, hash) = record.value();
                        Head { count, hash, length }
                    }
                    None => Head::EMPTY,
                };
                let mut finished = false;
                for _ in 0..LINES_PER_TRANSACTION {
                    let mut line = Vec::new();
                    if lines.read_until(b'\n', &mut line).map_err(|e| format!("index: reading the AOF: {e}"))? == 0 {
                        finished = true;
                        break;
                    }
                    if line.pop() != Some(b'\n') {
                        return Err(format!("index: the line at byte {} of the committed prefix has no terminator", prefix.length));
                    }
                    let id = event_id(&line);
                    let position = prefix.count + 1;
                    let place = (position, prefix.length, line.len() as u64);
                    if lines_table.insert((topic, id), place).map_err(store_error)?.is_some() {
                        return Err(format!("index: Event ID {} occurs twice in the topic", base64url(&id)));
                    }
                    prefix = Head { count: position, hash: id, length: prefix.length + line.len() as u64 + 1 };
                }
                prefix_table.insert(topic, (prefix.count, prefix.length, prefix.hash)).map_err(store_error)?;
                (prefix, finished)
            };
            transaction.commit().map_err(store_error)?;
            if finished {
                if prefix != *to {
                    return Err(format!(
                        "index disagrees with the sidecar: indexed {} entries, {} bytes, head {}; committed {} entries, {} bytes, head {}",
                        prefix.count,
                        prefix.length,
                        base64url(&prefix.hash),
                        to.count,
                        to.length,
                        base64url(&to.hash)
                    ));
                }
                return Ok(());
            }
        }
    }

    /// Bring the index up to the committed head `head` of the topic in
    /// `store`. The last line the index covers must still be in the file
    /// where the index says it is, and every line after it is read from the
    /// file and indexed.
    pub fn catch_up(&self, topic: &str, store: &TopicDir, head: &Head) -> Result<(), String> {
        let indexed = self.indexed(topic)?;
        if indexed.count > head.count || indexed.length > head.length {
            return Err(format!(
                "the index is ahead of the sidecar: {} entries, {} bytes indexed; {} entries, {} bytes committed",
                indexed.count, indexed.length, head.count, head.length
            ));
        }
        if indexed.count > 0 {
            let located = self.locate(topic, &indexed.hash)?.ok_or_else(|| format!("the index covers a head {} it has no line for", base64url(&indexed.hash)))?;
            let line = store.read_at(located.offset, located.length).map_err(|e| format!("reading the AOF: {e}"))?;
            if located.position != indexed.count || located.end() != indexed.length || event_id(&line) != indexed.hash {
                return Err(format!("the index disagrees with the AOF at position {}", indexed.count));
            }
        }
        let mut lines = store.read_range(indexed.length, head.length).map_err(|e| format!("reading the AOF: {e}"))?;
        self.extend(topic, &mut lines, head)
    }
}
