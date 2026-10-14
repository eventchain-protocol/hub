//! The heartbeat list, served as the specification appendix states:
//! `heartbeat/latest` is the latest heartbeat as one object, `heartbeat` is
//! the whole list as an array, and `heartbeat?from=&until=` the heartbeats
//! with `from <= issued_at < until`, an omitted bound open. Each statement is
//! served as the exact bytes the Hub signed and wrote.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::{internal, json_response, App};
use crate::heartbeat::{issued_at, Timestamp, Window};

pub async fn latest(Path(topic): Path<String>, State(app): State<Arc<App>>) -> Response {
    let topic = match app.topic(&topic) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    match topic.heartbeats.latest() {
        Some(statement) => json_response(StatusCode::OK, statement),
        None => (StatusCode::NOT_FOUND, "no heartbeat issued").into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    from: Option<String>,
    until: Option<String>,
}

pub async fn list(Path(topic): Path<String>, Query(range): Query<Range>, State(app): State<Arc<App>>) -> Response {
    let topic = match app.topic(&topic) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    let bound = |text: Option<String>| text.map(|t| Timestamp::parse(&t)).transpose().map_err(|e| (StatusCode::BAD_REQUEST, e).into_response());
    let window = match (bound(range.from), bound(range.until)) {
        (Ok(from), Ok(until)) => Window { from, until },
        (Err(response), _) | (_, Err(response)) => return response,
    };
    let id = topic.id.clone();
    let statements = match tokio::task::spawn_blocking(move || topic.heartbeats.all()).await.expect("reading the heartbeat list does not panic") {
        Ok(statements) => statements,
        Err(e) => return internal(format!("topic {id}: heartbeats.jsonl: {e}")),
    };
    let mut body = vec![b'['];
    for statement in statements {
        match issued_at(&statement) {
            Ok(at) if window.contains(&at) => {
                if body.len() > 1 {
                    body.push(b',');
                }
                body.extend_from_slice(&statement);
            }
            Ok(_) => {}
            Err(e) => return internal(format!("topic {id}: {e}")),
        }
    }
    body.push(b']');
    json_response(StatusCode::OK, body)
}
