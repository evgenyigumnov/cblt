use crate::error::CbltError;
use async_compression::tokio::write::GzipEncoder;
use http::{Request, Response, StatusCode};
use log::{debug, info};
use std::fmt::Debug;
use std::pin;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::instrument;

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn send_response_file<S>(
    mut socket: S,
    response: Response<impl AsyncRead + Debug + AsyncWrite>,
    req_opt: &Request<Vec<u8>>,
) -> Result<(), CbltError>
where
    S: AsyncWriteExt + Unpin,
{
    let (parts, mut b) = response.into_parts();
    let mut body = pin::pin!(b);

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
    let gzip_supported = gzip_support_detect(req_opt);
    if gzip_supported {
        // socket.write_all(b"Content-Encoding: gzip").await?;
        // socket.write_all(b"\r\n").await?;
    }

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

    if gzip_supported {
        debug!("Gzip supported");
        let gzip_stream = GzipEncoder::new(body);
        let mut gzip_reader = tokio::io::BufReader::new(gzip_stream);
        tokio::io::copy(&mut gzip_reader, &mut socket).await?;
    } else {
        tokio::io::copy(&mut body, &mut socket).await?;
    }

    // Ensure all data is flushed
    socket.flush().await?;

    Ok(())
}

fn gzip_support_detect(req_opt: &Request<Vec<u8>>) -> bool {
    let accept_encoding = req_opt
        .headers()
        .get(http::header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok());

    let gzip_supported = accept_encoding
        .map(|encodings| encodings.contains("gzip"))
        .unwrap_or(false);
    gzip_supported
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn send_response_stream<S, T>(
    mut socket: &mut S,
    response: Response<&str>,
    req_opt: &Request<Vec<u8>>,
    stream: &mut T,
) -> Result<(), CbltError>
where
    S: AsyncWriteExt + Unpin,
    T: futures_core::stream::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    let (parts, _) = response.into_parts();

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
    let gzip_supported = gzip_support_detect(req_opt);
    if gzip_supported {
        socket.write_all(b"Content-Encoding: gzip").await?;
        socket.write_all(b"\r\n").await?;
    }
    // Write headers without allocation
    for (key, value) in parts.headers.iter() {
        debug!("{}: {}", key.as_str(), value.to_str()?);
        socket.write_all(key.as_str().as_bytes()).await?;
        socket.write_all(b": ").await?;
        socket.write_all(value.as_bytes()).await?;
        socket.write_all(b"\r\n").await?;
    }

    // End headers
    socket.write_all(b"\r\n").await?;

    // Ensure all headers are flushed
    socket.flush().await?;

    use futures_util::stream::StreamExt;
    if gzip_supported {
        debug!("Gzip supported");

        let mut gzip_stream = GzipEncoder::new(&mut socket);

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            gzip_stream.write_all(&chunk).await?;
        }
        gzip_stream.shutdown().await?;
        gzip_stream.flush().await?;
    } else {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            socket.write_all(&chunk).await?;
        }
    }
    socket.flush().await?;

    Ok(())
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub fn log_request_response<T>(request: &Request<Vec<u8>>, status_code: StatusCode) {
    let method = &request.method();
    let uri = request.uri();
    let headers = request.headers();

    let host_header = headers
        .get("Host")
        .map_or("-", |v| v.to_str().unwrap_or("-"));

    info!(
        "Request: {} {} {} {}",
        method,
        uri,
        host_header,
        status_code.as_u16()
    );
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn send_response<S>(socket: &mut S, response: Response<Vec<u8>>) -> Result<(), CbltError>
where
    S: AsyncWriteExt + Unpin,
{
    let (parts, body) = response.into_parts();

    // Estimate capacity to reduce reallocations
    let mut resp_bytes = Vec::with_capacity(128 + body.len());
    resp_bytes.write_all(b"HTTP/1.1 ").await?;

    let mut itoa_buf = itoa::Buffer::new();
    let status_str = itoa_buf.format(parts.status.as_u16());
    resp_bytes.write_all(status_str.as_bytes()).await?;

    resp_bytes.write_all(b" ").await?;
    resp_bytes
        .write_all(parts.status.canonical_reason().unwrap_or("").as_bytes())
        .await?;
    resp_bytes.flush().await?;

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
