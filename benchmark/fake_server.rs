#!/usr/bin/env rust-script
//! Install `rust-script` with `cargo install rust-script` and run with:
//!
//! rust-script ./fake_server.rs
//!
//! ```cargo
//! [dependencies]
//! axum = "0.7.7"
//! serde_json = "1.0.132"
//! tokio = { version = "1.41.0", features = ["full"] }
//! ```


use axum::{routing::post, extract::Json, response::IntoResponse, Router};
use serde_json::Value;

#[tokio::main]
async fn main() {
    let app = Router::new().route("/*path", post(echo_handler)).route("/", post(echo_handler));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn echo_handler(Json(payload): Json<Value>) -> impl IntoResponse {
    println!("{:?}", payload);
    Json(payload)
}
