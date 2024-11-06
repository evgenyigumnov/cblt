use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use http::{Request, Response, StatusCode};
use std::error::Error;
use std::path::Path;
use tokio::fs;
use std::str;
use log::{debug, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("trace")).try_init();


    info!("cblt started");

    let listener = TcpListener::bind("0.0.0.0:80").await?;

    loop {
        let (mut socket, _) = listener.accept().await?;

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
                            let _ = send_response(&mut socket, response).await;
                            return;
                        }
                    };

                    let request = match parse_request(req_str) {
                        Some(req) => {
                            debug!("{:?}", req);
                            info!("Request: {} {} {}", req.method(), req.uri(), req.headers().get("Host").unwrap().to_str().unwrap());
                            req
                        },
                        None => {
                            info!("Bad request");
                            let response = Response::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(Vec::new())
                                .unwrap();
                            let _ = send_response(&mut socket, response).await;
                            return;
                        }
                    };

                    let host = request.headers().get("host").unwrap().to_str().unwrap();
                    if host != "example.com" {
                        let response = Response::builder()
                            .status(StatusCode::FORBIDDEN)
                            .body(Vec::new())
                            .unwrap();

                        info!("Forbidden request");
                        let _ = send_response(&mut socket, response).await;
                        return;
                    }

                    let mut path = ".".to_string();
                    path.push_str(request.uri().path());

                    let path = if Path::new(&path).is_dir() {
                        format!("{}/index.html", path)
                    } else {
                        path
                    };

                    match fs::read(&path).await {
                        Ok(contents) => {
                            let response = Response::builder()
                                .status(StatusCode::OK)
                                .header("Content-Length", contents.len())
                                .body(contents)
                                .unwrap();
                            let _ = send_response(&mut socket, response).await;
                        }
                        Err(_) => {
                            let response = Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Vec::new())
                                .unwrap();
                            let _ = send_response(&mut socket, response).await;
                        }
                    }
                }
            }
        });
    }
}

async fn send_response(socket: &mut tokio::net::TcpStream, response: Response<Vec<u8>>) -> Result<(), Box<dyn Error>> {
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
