use std::error::Error;
use http::{Request, Response, StatusCode};
use log::{debug, info};
use tokio::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

#[instrument]
pub async fn send_response_file(
    socket: &mut tokio::net::TcpStream,
    response: Response<impl AsyncReadExt + Unpin + std::fmt::Debug>,
    req_opt: Option<&Request<()>>,
) -> Result<(), Box<dyn Error>> {
    if let Some(req) = req_opt {
        debug!("{:?}", req);
        if let Some(host_header) = req.headers().get("Host") {
            info!(
                "Request: {} {} {} {}",
                req.method(),
                req.uri(),
                host_header.to_str().unwrap_or(""),
                response.status().as_u16()
            );
        } else {
            info!(
                "Request: {} {} {}",
                req.method(),
                req.uri(),
                response.status().as_u16()
            );
        }
    } else {
        info!("Response: {}", response.status().as_u16());
    }
    let (parts, mut body) = response.into_parts();

    // Build and send headers
    let mut headers = Vec::with_capacity(128);
    let status_line = format!(
        "HTTP/1.1 {} {}\r\n",
        parts.status.as_u16(),
        parts.status.canonical_reason().unwrap_or("")
    );
    headers.extend_from_slice(status_line.as_bytes());

    for (key, value) in parts.headers.iter() {
        headers.extend_from_slice(key.as_str().as_bytes());
        headers.extend_from_slice(b": ");
        headers.extend_from_slice(value.as_bytes());
        headers.extend_from_slice(b"\r\n");
    }

    headers.extend_from_slice(b"\r\n");
    socket.write_all(&headers).await?;

    // Stream the body
    io::copy(&mut body, socket).await?;

    Ok(())
}

#[instrument]
pub async fn send_response(socket: &mut tokio::net::TcpStream, response: Response<Vec<u8>>, req_opt: Option<&Request<()>>) -> Result<(), Box<dyn Error>> {
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


#[instrument]
pub fn error_response(status: StatusCode) -> Response<Vec<u8>> {
    let msg = match status {
        StatusCode::BAD_REQUEST => "Bad request",
        StatusCode::FORBIDDEN => "Forbidden",
        StatusCode::NOT_FOUND => "Not found",
        _ => "Unknown error",
    };

    Response::builder()
        .status(status)
        .body(msg.as_bytes().to_vec())
        .unwrap()
}

