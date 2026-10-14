//! Subscriptions: `POST /topics/{t}/subscriptions` with one absolute http
//! or https URL as the body. The URL is stored in its normal form, so the
//! same URL written two ways is one subscription. Subscribing needs the
//! topic's write role; what a subscriber then receives is the webhook
//! module's.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use super::{bearer, denied, internal, App};
use crate::access::Action;
use crate::topic::http_url;

pub async fn subscribe(Path(topic): Path<String>, State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    let topic = match app.topic(&topic) {
        Ok(topic) => topic,
        Err(response) => return response,
    };
    if let Err(e) = topic.access.authorise(bearer(&headers), Action::Subscribe) {
        return denied(e);
    }
    let url = match std::str::from_utf8(&body).map_err(|_| "the body is not UTF-8".to_string()).and_then(http_url) {
        Ok(url) => url,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("a subscription is one absolute http or https URL: {e}")).into_response(),
    };
    let id = topic.id.clone();
    let added = tokio::task::spawn_blocking(move || topic.subscriptions.add(url.as_str())).await.expect("subscribing does not panic");
    match added {
        Ok(true) => StatusCode::CREATED.into_response(),
        Ok(false) => (StatusCode::OK, "already subscribed").into_response(),
        Err(e) => internal(format!("topic {id}: subscriptions.jsonl: {e}")),
    }
}
