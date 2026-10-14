//! Payloads: `PUT` and `GET /topics/{t}/payloads/{ref}`. The body is
//! ciphertext the Hub cannot read and stores as it came; a ref is opaque and
//! names one file. The topic's access policy decides who may do either, and
//! a payload once put is never replaced: the same bytes again are a 200,
//! other bytes a 409.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::fs::File;
use tokio_util::io::ReaderStream;

use super::{bearer, denied, internal, App};
use crate::access::Action;
use crate::store::{is_safe_segment, Put};
use crate::topic::Topic;

/// The topic, once the request's token is allowed `action` on it and the ref is a safe segment.
fn admitted(app: &App, topic: &str, reference: &str, headers: &HeaderMap, action: Action) -> Result<Arc<Topic>, Response> {
    let topic = app.topic(topic)?;
    topic.access.authorise(bearer(headers), action).map_err(denied)?;
    if !is_safe_segment(reference) {
        return Err((StatusCode::BAD_REQUEST, "a payload ref is one non-empty path segment of printable ASCII, without .. and not .").into_response());
    }
    Ok(topic)
}

pub async fn put(Path((topic, reference)): Path<(String, String)>, State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    let topic = match admitted(&app, &topic, &reference, &headers, Action::WritePayload) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    let id = topic.id.clone();
    let put = tokio::task::spawn_blocking(move || topic.payloads.put(&reference, &body)).await.expect("putting a payload does not panic");
    match put {
        Ok(Put::Created) => StatusCode::CREATED.into_response(),
        Ok(Put::Identical) => StatusCode::OK.into_response(),
        Ok(Put::Conflict) => (StatusCode::CONFLICT, "the ref holds other bytes; a payload is never replaced").into_response(),
        Err(e) => internal(format!("topic {id}: storing a payload: {e}")),
    }
}

/// The exact bytes that were put, streamed.
pub async fn get(Path((topic, reference)): Path<(String, String)>, State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let topic = match admitted(&app, &topic, &reference, &headers, Action::ReadPayload) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    let path = topic.payloads.path(&reference);
    let opened = async {
        let file = File::open(&path).await?;
        let length = file.metadata().await?.len();
        Ok::<(File, u64), std::io::Error>((file, length))
    };
    match opened.await {
        Ok((file, length)) => {
            let body = Body::from_stream(ReaderStream::new(file));
            (StatusCode::OK, [(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::CONTENT_LENGTH, length.to_string())], body).into_response()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (StatusCode::NOT_FOUND, "no payload with that ref").into_response(),
        Err(e) => internal(format!("topic {}: reading payload {reference:?}: {e}", topic.id)),
    }
}
