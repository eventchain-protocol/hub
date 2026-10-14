//! Spike: a web server that signs whatever JSON is posted to `/` with the
//! eventchain library. Not conforming to the protocol; it exists to prove the
//! Rust-to-Haskell seam end to end.

mod acceptor;

use std::sync::Arc;

use axum::{body::Bytes, extract::State, http::StatusCode, routing::post, Json, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};

struct App {
    key: acceptor::Key,
}

fn key_from_env() -> acceptor::Key {
    let hex = std::env::var("HUB_KEY").expect("HUB_KEY must be set to 64 hex characters");
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("HUB_KEY must be hex"))
        .collect();
    let bytes: [u8; 32] = bytes.try_into().expect("HUB_KEY must be 32 bytes");
    acceptor::Key::load(bytes).unwrap_or_else(|e| panic!("HUB_KEY is not a P-256 private scalar: {e:?}"))
}

async fn sign(State(app): State<Arc<App>>, body: Bytes) -> Result<Json<Value>, (StatusCode, String)> {
    serde_json::from_slice::<Value>(&body).map_err(|e| (StatusCode::BAD_REQUEST, format!("body is not JSON: {e}")))?;
    let signature = app.key.sign(&body).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?;
    Ok(Json(json!({
        "signature": URL_SAFE_NO_PAD.encode(signature),
        "public_key": URL_SAFE_NO_PAD.encode(app.key.public_key()),
    })))
}

#[tokio::main]
async fn main() {
    let _runtime = acceptor::Runtime::start();
    let app = Arc::new(App { key: key_from_env() });
    let router = Router::new().route("/", post(sign)).with_state(app);
    let addr = std::env::var("HUB_ADDR").unwrap_or_else(|_| "127.0.0.1:3717".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| panic!("bind {addr}: {e}"));
    println!("listening on http://{addr}");
    axum::serve(listener, router).await.expect("serve");
}
