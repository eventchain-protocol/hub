//! The HTTP surface: the head of a topic and submission here, distribution
//! and the heartbeat list in the modules named for them.

mod distribute;
mod heartbeat;

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::acceptor::Key;
use crate::signer::{base64url, issued_at_now, Statement};
use crate::topic::{Backpressure, Outcome, Refusal, Topic};

pub struct App {
    pub key: Arc<Key>,
    pub topics: HashMap<String, Arc<Topic>>,
}

impl App {
    fn topic(&self, name: &str) -> Result<&Arc<Topic>, Response> {
        self.topics.get(name).ok_or_else(|| (StatusCode::NOT_FOUND, "no such topic").into_response())
    }
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/topics/{topic}/head", get(head))
        .route("/topics/{topic}/entries", get(distribute::extension).post(submit))
        .route("/topics/{topic}/aof", get(distribute::aof))
        .route("/topics/{topic}/lookup/{event_id}", get(distribute::lookup))
        .route("/topics/{topic}/heartbeat/latest", get(heartbeat::latest))
        .route("/topics/{topic}/heartbeat", get(heartbeat::list))
        .with_state(app)
}

async fn head(Path(topic): Path<String>, State(app): State<Arc<App>>) -> Response {
    let topic = match app.topic(&topic) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    let head = topic.head();
    let body = json!({"position": head.count.to_string(), "head_hash": base64url(&head.hash)});
    json_response(StatusCode::OK, serde_json::to_vec(&body).expect("a value serialises"))
}

async fn submit(Path(topic): Path<String>, State(app): State<Arc<App>>, body: Bytes) -> Response {
    let topic = match app.topic(&topic) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    let submission_hash: [u8; 32] = Sha256::digest(&body).into();
    let receiver = match topic.submit(body) {
        Ok(receiver) => receiver,
        Err(Backpressure::Full) => return (StatusCode::SERVICE_UNAVAILABLE, "the topic's writer is busy").into_response(),
        Err(Backpressure::Stopped) => return (StatusCode::INTERNAL_SERVER_ERROR, "the topic's writer has stopped").into_response(),
    };
    let Ok(outcome) = receiver.await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "the topic's writer stopped before answering").into_response();
    };
    let statement = |kind| Statement::new(kind, &topic.id).member("submission_hash", base64url(&submission_hash)).member("issued_at", issued_at_now());
    let (status, signed) = match outcome {
        Outcome::Accepted { position, head_hash, count } => {
            if count == 1 && head_hash != submission_hash {
                panic!("topic {}: a one-line submission's hash {} is not its Event ID {}", topic.id, base64url(&submission_hash), base64url(&head_hash));
            }
            let receipt = statement("receipt")
                .member("position", position.to_string())
                .member("head_hash", base64url(&head_hash))
                .member("count", count.to_string());
            (StatusCode::CREATED, receipt.sign(&app.key))
        }
        Outcome::Refused(Refusal::HeadMismatch { position, head_hash }) => {
            let refusal = statement("refusal")
                .member("reason", "head_mismatch")
                .member("position", position.to_string())
                .member("head_hash", base64url(&head_hash));
            (StatusCode::CONFLICT, refusal.sign(&app.key))
        }
        Outcome::Refused(Refusal::Line { line, fault }) => {
            let refusal = statement("refusal").member("reason", fault.name()).member("line", line.to_string());
            (StatusCode::CONFLICT, refusal.sign(&app.key))
        }
        Outcome::Failed(message) => return (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    };
    match signed {
        Ok(bytes) => json_response(status, bytes),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("the Hub key did not sign: {e:?}")).into_response(),
    }
}

fn json_response(status: StatusCode, bytes: Vec<u8>) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], bytes).into_response()
}

fn internal(message: impl Into<String>) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, message.into()).into_response()
}
