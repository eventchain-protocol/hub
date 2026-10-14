//! The HTTP surface of milestone 2: the head of a topic, and submission.

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
    pub key: Key,
    pub topics: HashMap<String, Topic>,
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/topics/{topic}/head", get(head))
        .route("/topics/{topic}/entries", axum::routing::post(submit))
        .with_state(app)
}

async fn head(Path(topic): Path<String>, State(app): State<Arc<App>>) -> Response {
    let Some(topic) = app.topics.get(&topic) else {
        return (StatusCode::NOT_FOUND, "no such topic").into_response();
    };
    let head = topic.head();
    let body = json!({"position": head.count.to_string(), "head_hash": base64url(&head.hash)});
    json_response(StatusCode::OK, serde_json::to_vec(&body).expect("a value serialises"))
}

async fn submit(Path(topic): Path<String>, State(app): State<Arc<App>>, body: Bytes) -> Response {
    let Some(topic) = app.topics.get(&topic) else {
        return (StatusCode::NOT_FOUND, "no such topic").into_response();
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
