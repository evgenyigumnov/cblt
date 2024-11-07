use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use http::{Request, Response, StatusCode};
use std::error::Error;
use std::path::{PathBuf};
use tokio::fs;
use std::str;
use log::{debug, info};
use std::sync::Arc;
use kdl::KdlDocument;
use crate::config::{build_config, Directive};
use bytes::Bytes;
use reqwest;

mod config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("info")).try_init();

    info!("Cblt started");

    // Read configuration from Cbltfile
    let cbltfile_content = fs::read_to_string("Cbltfile").await?;
    let doc: KdlDocument = cbltfile_content.parse()?;
    let config = Arc::new(build_config(&doc)?);

    let listener = TcpListener::bind("0.0.0.0:80").await?;

    loop {
        let (mut socket, _) = listener.accept().await?;
        let config = Arc::clone(&config);

        tokio::spawn(async move {
            let mut buf = [0; 4096];

            match socket.read(&mut buf).await {
                Ok(n) => {
                    let req_str = match str::from_utf8(&buf[..n]) {
                        Ok(v) => v,
                        Err(_) => {
                            let response = error_response(StatusCode::BAD_REQUEST);
                            let _ = send_response(&mut socket, response, None).await;
                            return;
                        }
                    };

                    let request = match parse_request(req_str) {
                        Some(req) => req,
                        None => {
                            let response = error_response(StatusCode::BAD_REQUEST);
                            let _ = send_response(&mut socket, response, None).await;
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
                            let _ = send_response(&mut socket, response, req_opt).await;
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

                                    match fs::read(&file_path).await {
                                        Ok(contents) => {
                                            let response = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Length", contents.len())
                                                .body(contents)
                                                .unwrap();
                                            let _ = send_response(&mut socket, response, req_opt).await;
                                            handled = true;
                                            break;
                                        }
                                        Err(_) => {
                                            let response = error_response(StatusCode::NOT_FOUND);
                                            let _ = send_response(&mut socket, response, req_opt).await;
                                            handled = true;
                                            break;
                                        }
                                    }
                                } else {
                                    let response = error_response(StatusCode::INTERNAL_SERVER_ERROR);
                                    let _ = send_response(&mut socket, response, req_opt).await;
                                    handled = true;
                                    break;
                                }
                            }
                            Directive::ReverseProxy { pattern, destination } => {
                                debug!("Reverse proxy: {} -> {}", pattern, destination);
                                if matches_pattern(pattern, request.uri().path()) {
                                    let dest_uri = format!("{}{}", destination, request.uri().path());
                                    debug!("Destination URI: {}", dest_uri);
                                    let client = reqwest::Client::new();
                                    let mut req_builder = client.request(
                                        request.method().clone(),
                                        &dest_uri,
                                    );

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
                                            let _ = send_response(&mut socket, response, req_opt).await;
                                            handled = true;
                                            break;
                                        }
                                        Err(_) => {
                                            let response = error_response(StatusCode::BAD_GATEWAY);
                                            let _ = send_response(&mut socket, response, req_opt).await;
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
                                    .body(dest.as_bytes().to_vec())
                                    .unwrap();
                                let _ = send_response(&mut socket, response, req_opt).await;
                                handled = true;
                                break;
                            }
                        }
                    }

                    if !handled {
                        let response = error_response(StatusCode::NOT_FOUND);
                        let _ = send_response(&mut socket, response, req_opt).await;
                    }
                }
                Err(_) => return,
            }
        });
    }
}

async fn send_response(socket: &mut tokio::net::TcpStream, response: Response<Vec<u8>>, req_opt: Option<&Request<()>>) -> Result<(), Box<dyn Error>> {
    if let Some(req) = req_opt {
        debug!("{:?}", req);
        if let Some(host_header) = req.headers().get("Host") {
            info!("Request: {} {} {} {}", req.method(), req.uri(), host_header.to_str().unwrap_or(""), response.status().as_u16());
        } else {
            info!("Request: {} {} {}", req.method(), req.uri(), response.status().as_u16());
        }
    } else {
        info!("Response: {}", response.status().as_u16());
    }
    let (parts, body) = response.into_parts();

    // Estimate capacity to reduce reallocations
    let mut resp_bytes = Vec::with_capacity(128 + body.len());
    let status_line = format!(
        "HTTP/1.1 {} {}\r\n",
        parts.status.as_u16(),
        parts.status.canonical_reason().unwrap_or("")
    );
    resp_bytes.extend_from_slice(status_line.as_bytes());

    for (key, value) in parts.headers.iter() {
        resp_bytes.extend_from_slice(key.as_str().as_bytes());
        resp_bytes.extend_from_slice(b": ");
        resp_bytes.extend_from_slice(value.as_bytes());
        resp_bytes.extend_from_slice(b"\r\n");
    }

    resp_bytes.extend_from_slice(b"\r\n");
    resp_bytes.extend_from_slice(&body);

    socket.write_all(&resp_bytes).await?;

    Ok(())
}

fn parse_request(req_str: &str) -> Option<Request<()>> {
    let mut lines = req_str.lines();

    // Parse the request line
    let mut request_line_parts = lines.next()?.split_whitespace();
    let method = request_line_parts.next()?;
    let uri = request_line_parts.next()?;
    let version_str = request_line_parts.next()?;
    if request_line_parts.next().is_some() {
        return None;
    }

    let version = match version_str {
        "HTTP/1.1" => http::Version::HTTP_11,
        "HTTP/1.0" => http::Version::HTTP_10,
        _ => return None,
    };

    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .version(version);

    // Parse headers
    for line in lines {
        if line.is_empty() {
            break;
        }
        let mut header_parts = line.splitn(2, ": ");
        let key = header_parts.next()?;
        let value = header_parts.next()?;
        builder = builder.header(key, value);
    }

    builder.body(()).ok()
}



fn error_response(status: StatusCode) -> Response<Vec<u8>> {

    let msg = match status {
        StatusCode::BAD_REQUEST => {
            "Bad request"
        }
        StatusCode::FORBIDDEN => {
            "Forbidden"
        }
        StatusCode::NOT_FOUND => {
            "Not found"
        }
        _ => {
            "Unknown error"
        }
    };

    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(str::as_bytes(msg).to_vec())
        .unwrap()
}
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
