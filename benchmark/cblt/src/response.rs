use http::header::TRANSFER_ENCODING;
use http::{HeaderValue, Request, Response, StatusCode};
use log::{debug, info};
use std::error::Error;
use std::fmt::Debug;
use tokio::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;
use tracing::instrument;

#[instrument(level = "trace", skip_all)]
pub async fn send_response_file(
    socket: &mut TcpStream,
    response: Response<impl AsyncReadExt + Unpin + Debug>,
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
    let (mut parts, mut body) = response.into_parts();

    // Add Transfer-Encoding: chunked header
    parts
        .headers
        .insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));

    // Wrap the socket in a BufWriter
    let mut writer = BufWriter::new(socket);

    // Write status line without allocation
    writer.write_all(b"HTTP/1.1 ").await?;
    let mut itoa_buf = itoa::Buffer::new();
    let status_str = itoa_buf.format(parts.status.as_u16());
    writer.write_all(status_str.as_bytes()).await?;
    writer.write_all(b" ").await?;
    writer
        .write_all(parts.status.canonical_reason().unwrap_or("").as_bytes())
        .await?;
    writer.write_all(b"\r\n").await?;

    // Write headers without allocation
    for (key, value) in parts.headers.iter() {
        writer.write_all(key.as_str().as_bytes()).await?;
        writer.write_all(b": ").await?;
        writer
            .write_all(value.to_str().unwrap_or("").as_bytes())
            .await?;
        writer.write_all(b"\r\n").await?;
    }

    // End headers
    writer.write_all(b"\r\n").await?;

    // Write body with chunked encoding
    write_chunked_body(&mut body, &mut writer).await?;

    // Ensure all data is flushed
    writer.flush().await?;

    Ok(())
}

const BUFFER_SIZE: usize = 8192;
const HEX_DIGITS: &[u8] = b"0123456789ABCDEF";

async fn write_chunked_body<R, W>(mut reader: R, writer: &mut W) -> io::Result<()>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut buf = [0u8; BUFFER_SIZE];
    let mut size_buf = [0u8; 16]; // Buffer for hex chunk size

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }

        // Write chunk size in hex without allocation
        let mut idx = size_buf.len() - 1;
        let mut size = n;
        // Convert to hex digits
        loop {
            size_buf[idx] = HEX_DIGITS[size % 16];
            size /= 16;
            if size == 0 {
                break;
            }
            idx -= 1;
        }
        writer.write_all(&size_buf[idx..]).await?;
        writer.write_all(b"\r\n").await?;

        // Write chunk data
        writer.write_all(&buf[..n]).await?;
        writer.write_all(b"\r\n").await?;
        writer.flush().await?;
    }

    // Write final chunk
    writer.write_all(b"0\r\n\r\n").await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn send_response(
    socket: &mut tokio::net::TcpStream,
    response: Response<Vec<u8>>,
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

#[instrument(level = "trace", skip_all)]
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
