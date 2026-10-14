//! One writer thread per topic owns the AOF file and the head. It judges a
//! submission one line at a time through the acceptor, the first line
//! against the head it holds, writes the whole submission or none of it,
//! and answers the handler that sent it.

use std::path::Path;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;

use bytes::Bytes;
use tokio::sync::oneshot;

use crate::acceptor::{self, Fault, Verdict};
use crate::store::{reconcile, Head, Reconcile, TopicDir};

/// How many submissions may wait for the writer before the handler answers 503.
const QUEUE_DEPTH: usize = 64;

pub struct Submission {
    pub body: Bytes,
    pub reply: oneshot::Sender<Outcome>,
}

pub enum Outcome {
    /// Written and durable. `position` is the count after the append.
    Accepted { position: u64, head_hash: [u8; 32], count: u64 },
    /// Nothing written, for a reason the submitter is told under the Hub's signature.
    Refused(Refusal),
    /// Nothing written, because the Hub could not judge: a 500, not a refusal.
    Failed(String),
}

pub enum Refusal {
    /// The first line was not signed against the current head. Carries the head to sign against now.
    HeadMismatch { position: u64, head_hash: [u8; 32] },
    /// A line failed one of the verifier's checks.
    Line { line: u64, fault: Fault },
}

pub enum Backpressure {
    Full,
    Stopped,
}

pub struct Topic {
    pub id: String,
    pub heartbeat_interval_seconds: u64,
    sender: SyncSender<Submission>,
    head: Arc<Mutex<Head>>,
}

impl Topic {
    /// Open the directory, reconcile the sidecar with the file, and start the writer.
    pub fn open(dir: &Path) -> Result<Topic, String> {
        let name = dir.file_name().and_then(|n| n.to_str()).ok_or("topic directory has no name")?.to_string();
        let (mut store, settings, sidecar) = TopicDir::open(dir).map_err(|e| format!("{name}: {e}"))?;
        if settings.topic_id != name {
            return Err(format!("{name}: topic.json names the topic {:?}", settings.topic_id));
        }
        let head = recover(&name, &mut store, sidecar.unwrap_or(Head::EMPTY))?;
        let published = Arc::new(Mutex::new(head));
        let (sender, receiver) = sync_channel(QUEUE_DEPTH);
        let writer_head = Arc::clone(&published);
        let id = name.clone();
        thread::Builder::new()
            .name(format!("writer:{name}"))
            .spawn(move || run(&id, store, head, receiver, writer_head))
            .map_err(|e| format!("{name}: cannot start the writer thread: {e}"))?;
        Ok(Topic { id: name, heartbeat_interval_seconds: settings.heartbeat_interval_seconds, sender, head: published })
    }

    /// The committed head as the writer last published it.
    pub fn head(&self) -> Head {
        *self.head.lock().expect("the head lock is not poisoned")
    }

    pub fn submit(&self, body: Bytes) -> Result<oneshot::Receiver<Outcome>, Backpressure> {
        let (reply, receiver) = oneshot::channel();
        match self.sender.try_send(Submission { body, reply }) {
            Ok(()) => Ok(receiver),
            Err(TrySendError::Full(_)) => Err(Backpressure::Full),
            Err(TrySendError::Disconnected(_)) => Err(Backpressure::Stopped),
        }
    }
}

/// The writer loop. An IO failure after a judgement is a hard error: the
/// thread stops with the message, every waiting and later submission fails,
/// and the operator reconciles the topic at the next start.
fn run(id: &str, mut store: TopicDir, mut head: Head, receiver: Receiver<Submission>, published: Arc<Mutex<Head>>) {
    for submission in receiver {
        let outcome = match judge(&head, &submission.body) {
            Err(outcome) => outcome,
            Ok((hash, count)) => {
                let length = store.append(&submission.body, head.length).unwrap_or_else(|e| panic!("topic {id}: append failed: {e}"));
                head = Head { count: head.count + count, hash, length };
                store.write_head(&head).unwrap_or_else(|e| panic!("topic {id}: head sidecar not written: {e}"));
                *published.lock().expect("the head lock is not poisoned") = head;
                Outcome::Accepted { position: head.count, head_hash: head.hash, count }
            }
        };
        let _ = submission.reply.send(outcome);
    }
}

/// Every line of the body through the acceptor, the first against `head`
/// and each later one against the hash of the line before it. Returns the
/// new head hash and the line count, or the outcome that stops the append.
fn judge(head: &Head, body: &[u8]) -> Result<([u8; 32], u64), Outcome> {
    let mut hash = head.hash;
    let mut count = 0u64;
    for (line_number, line) in body.split(|b| *b == b'\n').enumerate() {
        let line_number = line_number as u64;
        match acceptor::accept(head.count + line_number, &hash, line) {
            Ok(Verdict::Sound { event_id }) => hash = event_id,
            Ok(Verdict::Unsound(Fault::ChainMismatch)) if line_number == 0 => {
                return Err(Outcome::Refused(Refusal::HeadMismatch { position: head.count, head_hash: head.hash }));
            }
            Ok(Verdict::Unsound(fault)) | Ok(Verdict::Malformed(fault)) => {
                return Err(Outcome::Refused(Refusal::Line { line: line_number, fault }));
            }
            Ok(Verdict::Undecided(fault)) => {
                return Err(Outcome::Failed(format!("libcrypto gave no answer on line {line_number}: {}", fault.name())));
            }
            Err(e) => return Err(Outcome::Failed(format!("acceptor on line {line_number}: {e:?}"))),
        }
        count += 1;
    }
    Ok((hash, count))
}

/// Put the sidecar beside the file. A whole tail beyond the sidecar is the
/// crash window between append and sidecar: it is re-judged from the
/// sidecar's head and, if every line is Sound, committed. Anything else is
/// the operator's to look at; the Hub does not guess.
fn recover(name: &str, store: &mut TopicDir, committed: Head) -> Result<Head, String> {
    let file_length = store.aof_length().map_err(|e| format!("{name}: {e}"))?;
    let tail = if file_length > committed.length { store.read_tail(committed.length).map_err(|e| format!("{name}: {e}"))? } else { Vec::new() };
    match reconcile(committed.length, file_length, &tail) {
        Reconcile::Consistent => Ok(committed),
        Reconcile::FileShorter => Err(format!("{name}: inconsistent: the AOF is {file_length} bytes but the sidecar commits {}", committed.length)),
        Reconcile::TornTail => Err(format!("{name}: inconsistent: {} bytes beyond the sidecar do not end a line", tail.len())),
        Reconcile::Replay => {
            let body = &tail[..tail.len() - 1];
            match judge(&committed, body) {
                Ok((hash, count)) => {
                    let head = Head { count: committed.count + count, hash, length: file_length };
                    store.write_head(&head).map_err(|e| format!("{name}: head sidecar not written: {e}"))?;
                    Ok(head)
                }
                Err(Outcome::Refused(Refusal::HeadMismatch { .. })) => Err(format!("{name}: inconsistent: the line after the sidecar is not chained to its head")),
                Err(Outcome::Refused(Refusal::Line { line, fault })) => Err(format!("{name}: inconsistent: line {line} beyond the sidecar is {}", fault.name())),
                Err(Outcome::Failed(message)) => Err(format!("{name}: {message}")),
                Err(Outcome::Accepted { .. }) => unreachable!("judge never returns an acceptance as an error"),
            }
        }
    }
}
