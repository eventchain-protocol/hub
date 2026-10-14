//! Distribution: the extension after an Event ID, the whole AOF, and an
//! Event ID lookup, all as exact bytes of the file.
//!
//! Every answer stops at the length in the head the writer last published.
//! The writer only ever writes beyond that length and replaces the sidecar
//! atomically, so reading up to it from a separate handle needs no lock and
//! never shows a line that is not committed. A lookup is made before the
//! head is read, so the head is at least as new as the index entry.

use std::io::SeekFrom;
use std::path::Path as FilePath;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use super::{internal, json_response, App};
use crate::acceptor::GENESIS;
use crate::index::Located;
use crate::topic::Topic;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct After {
    after: String,
}

/// The JSON Lines tail after the line with this Event ID. The genesis head,
/// 32 zero bytes, is the head of the empty prefix, so the tail after it is
/// the whole committed file. An Event ID the topic does not have is a 404:
/// the Hub has no such line, and the Producer draws its own conclusion.
pub async fn extension(Path(topic): Path<String>, Query(After { after }): Query<After>, State(app): State<Arc<App>>) -> Response {
    let topic = match app.topic(&topic) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    let event_id = match event_id(&after) {
        Ok(event_id) => event_id,
        Err(response) => return response,
    };
    let start = if event_id == GENESIS {
        0
    } else {
        match committed_line(topic, &event_id).await {
            Ok(Some((located, _))) => located.end(),
            Ok(None) => return (StatusCode::NOT_FOUND, "no line with that Event ID").into_response(),
            Err(response) => return response,
        }
    };
    file_range(topic.aof_path(), start, topic.head().length).await
}

/// The whole committed file.
pub async fn aof(Path(topic): Path<String>, State(app): State<Arc<App>>) -> Response {
    match app.topic(&topic) {
        Ok(topic) => file_range(topic.aof_path(), 0, topic.head().length).await,
        Err(response) => response,
    }
}

/// The position of the line with this Event ID and the line itself, as a JSON string.
pub async fn lookup(Path((topic, event_id_text)): Path<(String, String)>, State(app): State<Arc<App>>) -> Response {
    let topic = match app.topic(&topic) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    let event_id = match event_id(&event_id_text) {
        Ok(event_id) => event_id,
        Err(response) => return response,
    };
    let (located, line) = match committed_line(topic, &event_id).await {
        Ok(Some(found)) => found,
        Ok(None) => return (StatusCode::NOT_FOUND, "no line with that Event ID").into_response(),
        Err(response) => return response,
    };
    let Ok(line) = String::from_utf8(line) else {
        return internal(format!("topic {}: the line at position {} is not UTF-8", topic.id, located.position));
    };
    let body = json!({"position": located.position.to_string(), "line": line});
    json_response(StatusCode::OK, serde_json::to_vec(&body).expect("a value serialises"))
}

/// An Event ID in the entry specification's encoding: base64url without padding of 32 bytes.
fn event_id(text: &str) -> Result<[u8; 32], Response> {
    URL_SAFE_NO_PAD
        .decode(text)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "an Event ID is 32 bytes in base64url without padding").into_response())
}

/// The committed line with this Event ID and where it sits. The index is not
/// trusted for the bytes: the line read from the file must hash to the Event
/// ID, or the answer is an error rather than a wrong line.
async fn committed_line(topic: &Topic, event_id: &[u8; 32]) -> Result<Option<(Located, Vec<u8>)>, Response> {
    let Some(located) = topic.locate(event_id).map_err(|e| internal(format!("topic {}: {e}", topic.id)))? else {
        return Ok(None);
    };
    if located.position > topic.head().count {
        return Ok(None);
    }
    let line = read_at(topic.aof_path(), located.offset, located.length).await.map_err(|e| internal(format!("topic {}: reading the AOF: {e}", topic.id)))?;
    if Sha256::digest(&line)[..] != event_id[..] {
        return Err(internal(format!("topic {}: the index places an Event ID at position {} on a line that does not hash to it", topic.id, located.position)));
    }
    Ok(Some((located, line)))
}

async fn read_at(path: &FilePath, offset: u64, length: u64) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path).await?;
    file.seek(SeekFrom::Start(offset)).await?;
    let mut bytes = vec![0u8; length as usize];
    file.read_exact(&mut bytes).await?;
    Ok(bytes)
}

/// Bytes `from..to` of the file, streamed, as JSON Lines.
async fn file_range(path: &FilePath, from: u64, to: u64) -> Response {
    let opened = async {
        let mut file = File::open(path).await?;
        file.seek(SeekFrom::Start(from)).await?;
        Ok::<File, std::io::Error>(file)
    };
    match opened.await {
        Ok(file) => {
            let body = Body::from_stream(ReaderStream::new(file.take(to - from)));
            (StatusCode::OK, [(header::CONTENT_TYPE, "application/jsonl".to_string()), (header::CONTENT_LENGTH, (to - from).to_string())], body).into_response()
        }
        Err(e) => internal(format!("reading {}: {e}", path.display())),
    }
}
