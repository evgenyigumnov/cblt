use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use http::{Request, Response, StatusCode};
use std::error::Error;
use std::path::Path;
use tokio::fs;
use std::str;
use log::{debug, info};
use std::collections::HashMap;
use std::sync::Arc;
use kdl::KdlDocument;

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
                let mut buf = [0; 1024];

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
                        Some(req) => {
                            req
                        },
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

                    let host_config = match config.hosts.get(host) {
                        Some(cfg) => cfg,
                        None => {
                            let response = Response::builder()
                                .status(StatusCode::FORBIDDEN)
                                .body(Vec::new())
                                .unwrap();
                            let _ = send_response(&mut socket, response, Some(request)).await;
                            return;
                        }
                    };

                    // Construct the path to the requested resource
                    let mut path = host_config.root.clone();
                    path.push_str(request.uri().path());

                    let path = if Path::new(&path).is_dir() {
                        format!("{}/index.html", path)
                    } else {
                        path
                    };

                    // Serve the file
                    match fs::read(&path).await {
                        Ok(contents) => {
                            let response = Response::builder()
                                .status(StatusCode::OK)
                                .header("Content-Length", contents.len())
                                .body(contents)
                                .unwrap();
                            let _ = send_response(&mut socket, response, Some(request)).await;
                        }
                        Err(_) => {
                            let response = Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Vec::new())
                                .unwrap();
                            let _ = send_response(&mut socket, response, Some(request)).await;
                        }
                    }
                }
                Err(_) => return,
            }
        });
    }
}

async fn send_response(socket: &mut tokio::net::TcpStream, response: Response<Vec<u8>>, req_opt: Option<Request<()>>) -> Result<(), Box<dyn Error>> {
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

struct Config {
    hosts: HashMap<String, HostConfig>,
}

struct HostConfig {
    root: String,
    pattern: String,
}

fn build_config(doc: &KdlDocument) -> Result<Config, Box<dyn Error>> {
    // example.com {
    //     root * "."
    //     file_server
    // }


    let mut hosts = HashMap::new();

    for node in doc.nodes() {
        let hostname = node.name().value().to_string();
        let mut root = String::new();
        let mut pattern = String::new();

        if let Some(children) = node.children() {
            for child_node in children.nodes() {
                let child_name = child_node.name().value();

                if child_name == "root" {
                    let args = child_node
                        .entries()
                        .iter()
                        .filter_map(|e| e.value().as_string())
                        .collect::<Vec<&str>>();
                    if !args.is_empty() {
                        root = args[args.len() - 1].to_string();
                        pattern = args[args.len() - 2].to_string();
                    } else {
                        return Err(format!("No root path specified for host {}", hostname).into());
                    }
                }
            }
        }

        if root.is_empty() {
            return Err(format!("No root specified for host {}", hostname).into());
        }

        hosts.insert(hostname, HostConfig { root, pattern});
    }

    Ok(Config { hosts })
}
