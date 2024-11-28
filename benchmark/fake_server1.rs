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


use axum::{routing::post,routing::get, extract::Json, response::IntoResponse, Router};
use axum::body::Body;
use axum::extract::Request;
#[tokio::main]
async fn main() {
    let app = Router::new().route("/http/echo", post(echo_handler)).route("/http/echo",get(echo_handler)).route("/health", get(health_handler));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8081")
        .await
        .unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn echo_handler(req: Request<Body>) -> impl IntoResponse {
    println!("Echo request");
    "ok"
}

async fn health_handler() -> impl IntoResponse {
    println!("Health check");
    "ok"
}
