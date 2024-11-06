use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use http::{Request, Response, StatusCode};
use std::error::Error;
use std::path::Path;
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
    let _ = env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("trace")).try_init();

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
                            let response = Response::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(Vec::new())
                                .unwrap();
                            let _ = send_response(&mut socket, response, None).await;
                            return;
                        }
                    };

                    let request = match parse_request(req_str) {
                        Some(req) => req,
                        None => {
                            let response = Response::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(Vec::new())
                                .unwrap();
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
                            let response = Response::builder()
                                .status(StatusCode::FORBIDDEN)
                                .body(Vec::new())
                                .unwrap();
                            let _ = send_response(&mut socket, response, req_opt).await;
                            return;
                        }
                    };

                    let mut root_path = None;
                    let mut handled = false;

                    for directive in &host_config.directives {
                        match directive {
                            Directive::Root { pattern, path } => {
                                if matches_pattern(pattern, request.uri().path()) {
                                    root_path = Some(path.clone());
                                }
                            }
                            Directive::FileServer => {
                                if let Some(root) = &root_path {
                                    let mut file_path = root.clone();
                                    file_path.push_str(request.uri().path());

                                    let file_path = if Path::new(&file_path).is_dir() {
                                        format!("{}/index.html", file_path)
                                    } else {
                                        file_path
                                    };

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
                                            let response = Response::builder()
                                                .status(StatusCode::NOT_FOUND)
                                                .body(Vec::new())
                                                .unwrap();
                                            let _ = send_response(&mut socket, response, req_opt).await;
                                            handled = true;
                                            break;
                                        }
                                    }
                                } else {
                                    let response = Response::builder()
                                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                                        .body(Vec::new())
                                        .unwrap();
                                    let _ = send_response(&mut socket, response, req_opt).await;
                                    handled = true;
                                    break;
                                }
                            }
                            Directive::ReverseProxy { pattern, destination } => {
                                if matches_pattern(pattern, request.uri().path()) {
                                    let dest_uri = format!("{}{}", destination, request.uri().path());

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
                                            let response = Response::builder()
                                                .status(StatusCode::BAD_GATEWAY)
                                                .body(Vec::new())
                                                .unwrap();
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
                                    .header("Location", dest)
                                    .body(Vec::new())
                                    .unwrap();
                                let _ = send_response(&mut socket, response, req_opt).await;
                                handled = true;
                                break;
                            }
                        }
                    }

                    if !handled {
                        let response = Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Vec::new())
                            .unwrap();
                        let _ = send_response(&mut socket, response, req_opt).await;
                    }
                }
                Err(_) => return,
            }
        });
    }
}

async fn send_response(socket: &mut tokio::net::TcpStream, response: Response<Vec<u8>>, req_opt: Option<&Request<()>>) -> Result<(), Box<dyn Error>> {
    match req_opt {
        None => {
            match response.status() {
                StatusCode::BAD_REQUEST => {
                    info!("Bad request");
                }
                StatusCode::FORBIDDEN => {
                    info!("Forbidden");
                }
                StatusCode::NOT_FOUND => {
                    info!("Not found");
                }
                _ => {}
            }
        }
        Some(req) => {
            debug!("{:?}", req);
            if let Some(host_header) = req.headers().get("Host") {
                info!("Request: {} {} {} {}", req.method(), req.uri(), host_header.to_str().unwrap_or(""), response.status().as_u16());
            } else {
                info!("Request: {} {} {}", req.method(), req.uri(), response.status().as_u16());
            }
        }
    }

    let mut resp_bytes = Vec::new();
    let (parts, body) = response.into_parts();

    resp_bytes.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", parts.status.as_u16(), parts.status.canonical_reason().unwrap_or("")).as_bytes());

    for (key, value) in parts.headers.iter() {
        resp_bytes.extend_from_slice(format!("{}: {}\r\n", key, value.to_str()?).as_bytes());
    }

    resp_bytes.extend_from_slice(b"\r\n");

    resp_bytes.extend_from_slice(&body);

    socket.write_all(&resp_bytes).await?;

    Ok(())
}

fn parse_request(req_str: &str) -> Option<Request<()>> {
    let mut lines = req_str.lines();

    let request_line = lines.next()?.split_whitespace().collect::<Vec<&str>>();
    if request_line.len() != 3 {
        return None;
    }

    let method = request_line[0];
    let uri = request_line[1];
    let version = match request_line[2] {
        "HTTP/1.1" => http::Version::HTTP_11,
        "HTTP/1.0" => http::Version::HTTP_10,
        _ => return None,
    };

    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .version(version);

    for line in lines {
        if line.is_empty() {
            break;
        }
        let parts = line.splitn(2, ": ").collect::<Vec<&str>>();
        if parts.len() != 2 {
            return None;
        }
        builder = builder.header(parts[0], parts[1]);
    }

    builder.body(()).ok()
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
