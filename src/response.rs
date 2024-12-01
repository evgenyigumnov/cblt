use crate::error::CbltError;
use async_compression::tokio::write::GzipEncoder;
use bytes::BytesMut;
use http::{Request, Response, StatusCode};
use log::{debug, info};
use std::fmt::Debug;
use std::path::PathBuf;
use std::pin;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
#[cfg(feature = "trace")]
use tracing::instrument;

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub async fn send_response_file<S>(
    mut socket: S,
    response: Response<impl AsyncRead + Debug + AsyncWrite>,
    req: &Request<BytesMut>,
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
    let gzip_supported = gzip_support_detect(req);
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
        #[cfg(debug_assertions)]
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

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub async fn ranged_file_response(
    file: File,
    file_path: &PathBuf,
    file_size: u64,
    range: (u64, u64),
) -> Result<Response<File>, CbltError> {
    let (start, end) = range;
    let content_length = end - start + 1;

    // Seek to the start position
    let mut file = file;
    use tokio::io::AsyncSeekExt;
    file.seek(std::io::SeekFrom::Start(start)).await?;

    let mut content_range: heapless::String<200> = heapless::String::new();
    content_range
        .push_str("bytes ")
        .map_err(|_| CbltError::HeaplessError {})?;
    content_range
        .push_str(start.to_string().as_str())
        .map_err(|_| CbltError::HeaplessError {})?;
    content_range
        .push_str("-")
        .map_err(|_| CbltError::HeaplessError {})?;
    content_range
        .push_str(end.to_string().as_str())
        .map_err(|_| CbltError::HeaplessError {})?;
    content_range
        .push_str("/")
        .map_err(|_| CbltError::HeaplessError {})?;
    content_range
        .push_str(file_size.to_string().as_str())
        .map_err(|_| CbltError::HeaplessError {})?;

    let mime_type = mime_guess::from_path(file_path)
        .first_or_octet_stream()
        .to_string();
    Ok(Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header("Content-Length", content_length)
        .header("Content-Range", content_range.as_str())
        .header("Content-Type", mime_type)
        .body(file)?)
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn gzip_support_detect(req_opt: &Request<BytesMut>) -> bool {
    let accept_encoding = req_opt
        .headers()
        .get(http::header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok());

    accept_encoding
        .map(|encodings| encodings.contains("gzip"))
        .unwrap_or(false)
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub fn log_request_response(request: &Request<BytesMut>, status_code: StatusCode) {
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

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub async fn send_response<S>(socket: &mut S, response: Response<BytesMut>) -> Result<(), CbltError>
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
    resp_bytes.write_all(b"\r\n").await?;

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

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub fn error_response(status: StatusCode) -> Result<Response<BytesMut>, CbltError> {
    let msg = match status {
        StatusCode::BAD_REQUEST => "Bad request",
        StatusCode::FORBIDDEN => "Forbidden",
        StatusCode::NOT_FOUND => "Not found",
        StatusCode::METHOD_NOT_ALLOWED => "Method not allowed",
        StatusCode::INTERNAL_SERVER_ERROR => "Internal server error",
        StatusCode::BAD_GATEWAY => "Bad gateway",
        _ => "Unknown error",
    };
    let bytes = BytesMut::from(msg);
    Ok(Response::builder().status(status).body(bytes)?)
}
