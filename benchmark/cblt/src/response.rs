use http::{Request, Response, StatusCode};
use log::{debug, info};
use std::error::Error;
use std::fmt::Debug;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::instrument;

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn send_response_file(
    socket: &mut TcpStream,
    response: Response<impl AsyncReadExt + Unpin + Debug>,
    req_opt: Option<&Request<()>>,
) -> Result<(), Box<dyn Error>> {
    #[cfg(debug_assertions)]
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

    // Write status line without allocation
    socket.write_all(b"HTTP/1.1 ").await?;
    let mut itoa_buf = itoa::Buffer::new();
    let status_str = itoa_buf.format(parts.status.as_u16());
    socket.write_all(status_str.as_bytes()).await?;
    socket.write_all(b" ").await?;
    socket
        .write_all(parts.status.canonical_reason().unwrap_or("").as_bytes())
        .await?;
    socket.write_all(b"\r\n").await?;

    // Write headers without allocation
    for (key, value) in parts.headers.iter() {
        socket.write_all(key.as_str().as_bytes()).await?;
        socket.write_all(b": ").await?;
        socket.write_all(value.as_bytes()).await?;
        socket.write_all(b"\r\n").await?;
    }

    // End headers
    socket.write_all(b"\r\n").await?;

    // Ensure all headers are flushed
    socket.flush().await?;

    // Copy the body to the socket
    tokio::io::copy(&mut body, socket).await?;

    // Ensure all data is flushed
    socket.flush().await?;

    Ok(())
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn send_response(
    socket: &mut tokio::net::TcpStream,
    response: Response<Vec<u8>>,
    req_opt: Option<&Request<()>>,
) -> Result<(), Box<dyn Error>> {
    #[cfg(debug_assertions)]
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

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
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
