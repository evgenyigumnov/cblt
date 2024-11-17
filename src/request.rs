use crate::error::CbltError;
use bytes::BytesMut;
use http::Version;
use http::{Request, StatusCode};
use httparse::Status;
use log::debug;
use std::str;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

pub const BUF_SIZE: usize = 8192;
pub const HEADER_BUF_SIZE: usize = 32;
#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn socket_to_request<S>(
    socket: &mut S,
    mut buf: &mut BytesMut,
) -> Result<Request<BytesMut>, CbltError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    loop {
        let bytes_read = socket.read_buf(&mut buf).await.unwrap_or(0);
        if bytes_read == 0 {
            break;
        }
        // Try to parse the headers
        let mut headers = [httparse::EMPTY_HEADER; HEADER_BUF_SIZE];
        let mut req = httparse::Request::new(&mut headers);

        match req.parse(&buf) {
            Ok(Status::Complete(header_len)) => {
                let (request, _) = match parse_request_headers(header_len, &mut buf, socket).await?
                {
                    Some((req, content_length)) => (req, content_length),
                    None => {
                        return Err(CbltError::RequestError {
                            details: "Bad request".to_string(),
                            status_code: StatusCode::BAD_REQUEST,
                        });
                    }
                };

                #[cfg(debug_assertions)]
                debug!("{:?}", request);
                return Ok(request);
            }
            Ok(Status::Partial) => {
                // Need to read more data
                continue;
            }
            Err(_) => {
                return Err(CbltError::RequestError {
                    details: "Bad request".to_string(),
                    status_code: StatusCode::BAD_REQUEST,
                });
            }
        }
    }

    return Err(CbltError::ResponseError {
        details: "Bad request".to_string(),
        status_code: StatusCode::BAD_REQUEST,
    });
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn parse_request_headers<S>(
    header_len: usize,
    buf: &mut BytesMut,
    socket: &mut S,
) -> Result<Option<(Request<BytesMut>, Option<usize>)>, CbltError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let req_str = match str::from_utf8(&buf[..header_len]) {
        Ok(v) => v,
        Err(_) => {
            return Err(CbltError::RequestError {
                details: "Bad request".to_string(),
                status_code: StatusCode::BAD_REQUEST,
            });
        }
    };
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);

    match req.parse(req_str.as_bytes()) {
        Ok(Status::Complete(_)) => {
            let method = req.method.ok_or(CbltError::RequestError {
                details: "Bad request".to_string(),
                status_code: StatusCode::BAD_REQUEST,
            })?;
            let path = req.path.ok_or(CbltError::RequestError {
                details: "Bad request".to_string(),
                status_code: StatusCode::BAD_REQUEST,
            })?;
            let version = match req.version.ok_or(CbltError::RequestError {
                details: "Bad request".to_string(),
                status_code: StatusCode::BAD_REQUEST,
            })? {
                0 => Version::HTTP_10,
                1 => Version::HTTP_11,
                _ => return Ok(None),
            };

            let mut builder = Request::builder().method(method).uri(path).version(version);

            let mut content_length_opt = None;

            for header in req.headers.iter() {
                let name = header.name;
                let value = header.value;
                builder = builder.header(name, value);

                if name.eq_ignore_ascii_case("Content-Length") {
                    if let Ok(s) = std::str::from_utf8(value) {
                        if let Ok(len) = s.trim().parse::<usize>() {
                            content_length_opt = Some(len);
                        }
                    }
                }
            }

            if let Some(content_length) = content_length_opt {
                let mut body = buf.split_off(header_len);

                while body.len() < content_length {
                    let bytes_read = socket.read_buf(&mut body).await.unwrap_or(0);
                    if bytes_read == 0 {
                        break;
                    }
                }

                Ok(builder.body(body).ok().map(|req| (req, content_length_opt)))
            } else {
                Ok(builder
                    .body(BytesMut::new())
                    .ok()
                    .map(|req| (req, content_length_opt)))
            }
        }
        Ok(Status::Partial) => Ok(None), // Incomplete request
        Err(_) => Ok(None),              // Parsing failed
    }
}
