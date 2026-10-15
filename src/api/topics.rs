//! Topic creation: `POST /topics` with a body that is exactly a
//! `topic.json`. Only the operator's token may create a topic. The topic is
//! opened as one found at startup is, its heartbeats start, and it joins the
//! running set. The settings are all the Hub records: the binding between a
//! topic and its first Entry is open in the paper and not invented here.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use super::{bearer, denied, internal, App};
use crate::topic::{NotCreated, Topic};

pub async fn create(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(e) = app.operator.authorise(bearer(&headers)) {
        return denied(e);
    }
    let (data, index) = (app.data.clone(), Arc::clone(&app.index));
    let created = tokio::task::spawn_blocking(move || Topic::create(&data, &body, index)).await.expect("creating a topic does not panic");
    let topic = match created {
        Ok(topic) => Arc::new(topic),
        Err(NotCreated::Invalid(e)) => return (StatusCode::BAD_REQUEST, e).into_response(),
        Err(NotCreated::Exists) => return (StatusCode::CONFLICT, "the topic's directory exists").into_response(),
        Err(NotCreated::Failed(e)) => return internal(e),
    };
    let replaced = app.topics.write().expect("the topics lock is not poisoned").insert(topic.id.clone(), Arc::clone(&topic));
    if replaced.is_some() {
        panic!("topic {}: created over a running topic, whose directory did not exist", topic.id);
    }
    println!("topic {}: created, heartbeat every {} s to {} hosts", topic.id, topic.heartbeat_interval_seconds, topic.heartbeat_hosts.len());
    app.heartbeats.start(topic);
    StatusCode::CREATED.into_response()
}
