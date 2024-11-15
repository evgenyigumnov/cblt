use crate::buffer_pool::SmartVector;
use crate::error::CbltError;
use http::Version;
use http::{Request, StatusCode};
use httparse::Status;
use log::debug;
use std::str;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

pub const BUF_SIZE: usize = 8192;
pub const STATIC_BUF_SIZE: usize = 1024;
pub const HEADER_BUF_SIZE: usize = 32;
#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn socket_to_request<S>(
    socket: &mut S,
    buffer: SmartVector,
) -> Result<Request<Vec<u8>>, CbltError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let mut buf = buffer.lock().await;
    loop {
        let mut temp_buf = [0; STATIC_BUF_SIZE];
        let bytes_read = socket.read(&mut temp_buf).await.unwrap_or(0);
        if bytes_read == 0 {
            break; // Connection closed
        }
        buf.extend_from_slice(&temp_buf[..bytes_read]);

        // Try to parse the headers
        let mut headers = [httparse::EMPTY_HEADER; HEADER_BUF_SIZE];
        let mut req = httparse::Request::new(&mut headers);

        match req.parse(&buf) {
            Ok(Status::Complete(header_len)) => {
                // Headers parsed successfully
                let req_str = match str::from_utf8(&buf[..header_len]) {
                    Ok(v) => v,
                    Err(_) => {
                        return Err(CbltError::RequestError {
                            details: "Bad request".to_string(),
                            status_code: StatusCode::BAD_REQUEST,
                        });
                    }
                };

                // Parse the request headers and get Content-Length
                let (mut request, content_length) = match parse_request_headers(req_str) {
                    Some((req, content_length)) => (req, content_length),
                    None => {
                        return Err(CbltError::RequestError {
                            details: "Bad request".to_string(),
                            status_code: StatusCode::BAD_REQUEST,
                        });
                    }
                };

                if let Some(content_length) = content_length {
                    let mut body = buf[header_len..].to_vec();
                    let mut temp_buf = vec![0; content_length - body.len()];

                    while body.len() < content_length {
                        let bytes_read = socket.read(&mut temp_buf).await.unwrap_or(0);
                        if bytes_read == 0 {
                            break;
                        }
                        body.extend_from_slice(&temp_buf[..bytes_read]);
                    }

                    *request.body_mut() = body;
                }
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
pub fn parse_request_headers(req_str: &str) -> Option<(Request<Vec<u8>>, Option<usize>)> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);

    match req.parse(req_str.as_bytes()) {
        Ok(Status::Complete(_)) => {
            let method = req.method?;
            let path = req.path?;
            let version = match req.version? {
                0 => Version::HTTP_10,
                1 => Version::HTTP_11,
                _ => return None,
            };

            let mut builder = Request::builder().method(method).uri(path).version(version);

            let mut content_length = None;

            for header in req.headers.iter() {
                let name = header.name;
                let value = header.value;
                builder = builder.header(name, value);

                if name.eq_ignore_ascii_case("Content-Length") {
                    if let Ok(s) = std::str::from_utf8(value) {
                        if let Ok(len) = s.trim().parse::<usize>() {
                            content_length = Some(len);
                        }
                    }
                }
            }

            builder
                .body(Vec::new())
                .ok()
                .map(|req| (req, content_length))
        }
        Ok(Status::Partial) => None, // Incomplete request
        Err(_) => None,              // Parsing failed
    }
}

#[cfg(test)]
mod tests {
    use crate::only_in_debug;
    use crate::request::parse_request_headers;
    use std::error::Error;

    #[test]
    fn test_simple() -> Result<(), Box<dyn Error>> {
        only_in_debug();

        let request_str = "POST /submit HTTP/1.1\r\n\
Host: example.com\r\n\
User-Agent: curl/7.68.0\r\n\
Accept: */*\r\n\
Content-Type: application/json\r\n\
Content-Length: 15\r\n\r\n\
{\"key\":\"value\"}";

        let req = parse_request_headers(request_str);
        println!("{:#?}", req);

        Ok(())
    }
}
