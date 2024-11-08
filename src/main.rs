use crate::config::{build_config, Directive};
use crate::request::parse_request;
use crate::response::{error_response, send_response, send_response_file};
use bytes::Bytes;
use http::{Response, StatusCode};
use kdl::KdlDocument;
use log::{debug, info};
use reqwest;
use std::error::Error;
use std::path::PathBuf;
use std::str;
use std::sync::Arc;
use tokio::fs;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tracing::{instrument, span, Level};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::FmtSubscriber;

mod config;
mod request;
mod response;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    info!("Cblt started");
    #[cfg(debug_assertions)]
    only_in_debug();
    #[cfg(not(debug_assertions))]
    only_in_production();
    // Read configuration from Cbltfile
    let cbltfile_content = fs::read_to_string("Cbltfile").await?;
    let doc: KdlDocument = cbltfile_content.parse()?;
    let config = Arc::new(build_config(&doc)?);

    let listener = TcpListener::bind("0.0.0.0:80").await?;

    loop {
        let (mut socket, _) = listener.accept().await?;
        let config = Arc::clone(&config);

        tokio::spawn(async move {
            directive_process(&mut socket, config).await;
        });
    }
}

#[instrument(level = "trace", skip_all)]
async fn directive_process(socket: &mut tokio::net::TcpStream, config: Arc<config::Config>) {
    let mut buf = Vec::with_capacity(4096);
    let mut reader = BufReader::new(&mut *socket);
    let mut n = 0;
    loop {
        let bytes_read = reader.read_until(b'\n', &mut buf).await.unwrap();
        n += bytes_read;
        if bytes_read == 0 {
            break; // Connection closed
        }
        if buf.ends_with(b"\r\n\r\n") {
            break; // End of headers
        }
    }

    let req_str = match str::from_utf8(&buf[..n]) {
        Ok(v) => v,
        Err(_) => {
            let response = error_response(StatusCode::BAD_REQUEST);
            let _ = send_response(socket, response, None).await;
            return;
        }
    };

    let request = match parse_request(req_str) {
        Some(req) => req,
        None => {
            let response = error_response(StatusCode::BAD_REQUEST);
            let _ = send_response(socket, response, None).await;
            return;
        }
    };

    let host = match request.headers().get("Host") {
        Some(h) => h.to_str().unwrap_or(""),
        None => "",
    };

    let req_opt = Some(&request);

    let host_config = match config.hosts.get(host) {
        Some(cfg) => cfg,
        None => {
            let response = error_response(StatusCode::FORBIDDEN);
            let _ = send_response(socket, response, req_opt).await;
            return;
        }
    };

    let mut root_path = None;
    let mut handled = false;

    for directive in &host_config.directives {
        match directive {
            Directive::Root { pattern, path } => {
                debug!("Root: {} -> {}", pattern, path);
                if matches_pattern(pattern, request.uri().path()) {
                    root_path = Some(path.clone());
                }
            }
            Directive::FileServer => {
                debug!("File server");

                if let Some(root) = &root_path {
                    let mut file_path = PathBuf::from(root);
                    file_path.push(request.uri().path().trim_start_matches('/'));

                    if file_path.is_dir() {
                        file_path.push("index.html");
                    }

                    match File::open(&file_path).await {
                        Ok(file) => {

                            let metadata = file.metadata().await.unwrap();
                            let content_length = metadata.len();

                            let response = Response::builder()
                                .status(StatusCode::OK)
                                .header("Content-Length", content_length)
                                .body(file)
                                .unwrap();
                            let _ = send_response_file(&mut *socket, response, req_opt).await;
                            handled = true;
                            break;
                        }
                        Err(_) => {
                            let response = error_response(StatusCode::NOT_FOUND);
                            let _ = send_response(socket, response, req_opt).await;
                            handled = true;
                            break;
                        }
                    }
                } else {
                    let response = error_response(StatusCode::INTERNAL_SERVER_ERROR);
                    let _ = send_response(socket, response, req_opt).await;
                    handled = true;
                    break;
                }
            }
            Directive::ReverseProxy {
                pattern,
                destination,
            } => {
                debug!("Reverse proxy: {} -> {}", pattern, destination);
                if matches_pattern(pattern, request.uri().path()) {
                    let dest_uri = format!("{}{}", destination, request.uri().path());
                    debug!("Destination URI: {}", dest_uri);
                    let client = reqwest::Client::new();
                    let mut req_builder = client.request(request.method().clone(), &dest_uri);

                    for (key, value) in request.headers().iter() {
                        req_builder = req_builder.header(key, value);
                    }

                    match req_builder.send().await {
                        Ok(resp) => {
                            let status = resp.status();
                            let headers = resp.headers().clone();
                            let body = resp.bytes().await.unwrap_or_else(|_| Bytes::new());

                            let mut response_builder = Response::builder().status(status);

                            for (key, value) in headers.iter() {
                                response_builder = response_builder.header(key, value);
                            }

                            let response = response_builder.body(body.to_vec()).unwrap();
                            let _ = send_response(socket, response, req_opt).await;
                            handled = true;
                            break;
                        }
                        Err(_) => {
                            let response = error_response(StatusCode::BAD_GATEWAY);
                            let _ = send_response(socket, response, req_opt).await;
                            handled = true;
                            break;
                        }
                    }
                }
            }
            Directive::Redir { destination } => {
                let dest = destination.replace("{uri}", request.uri().path());
                let response = Response::builder()
                    .status(StatusCode::FOUND)
                    .header("Location", &dest)
                    .body(Vec::new()) // Empty body for redirects
                    .unwrap();
                let _ = send_response(socket, response, req_opt).await;
                handled = true;
                break;
            }
        }
    }

    if !handled {
        let response = error_response(StatusCode::NOT_FOUND);
        let _ = send_response(socket, response, req_opt).await;
    }
}

#[allow(dead_code)]
pub fn only_in_debug() {
    let _ =
        env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("debug")).try_init();
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::TRACE) // Set the maximum log level
        .with_span_events(FmtSpan::CLOSE)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set subscriber");
}

#[allow(dead_code)]
fn only_in_production() {
    let _ =
        env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("info")).try_init();
}

#[instrument(level = "trace", skip_all)]
fn matches_pattern(pattern: &str, path: &str) -> bool {
    if pattern == "*" {
        true
    } else if pattern.ends_with("*") {
        let prefix = &pattern[..pattern.len() - 1];
        path.starts_with(prefix)
    } else {
        pattern == path
    }
}
