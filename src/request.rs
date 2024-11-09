use std::str;
use http::{Request, StatusCode};
use tracing::instrument;
use http::Version;
use httparse::Status;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use crate::response::{error_response, send_response};


#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn socket_to_request<S>(socket: &mut S) -> Option<Request<()>>
    where S: AsyncReadExt + AsyncWriteExt + Unpin
{
    let mut buf = Vec::with_capacity(4096);
    let mut reader = BufReader::new(&mut *socket);
    let mut n = 0;
    loop {
        let bytes_read = reader.read_until(b'\n', &mut buf).await.unwrap();
        n += bytes_read;
        if bytes_read == 0 {
            break; // Connection closed
        }
        if buf.ends_with(b"\r\n\r\n") {
            break; // End of headers
        }
    }

    let req_str = match str::from_utf8(&buf[..n]) {
        Ok(v) => v,
        Err(_) => {
            let response = error_response(StatusCode::BAD_REQUEST);
            let _ = send_response(socket, response, None).await;
            return None;
        }
    };

    let request = match parse_request(req_str) {
        Some(req) => req,
        None => {
            let response = error_response(StatusCode::BAD_REQUEST);
            let _ = send_response(socket, response, None).await;
            return None;
        }
    };

    Some(request)
}



#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub fn parse_request(req_str: &str) -> Option<Request<()>> {
    let mut headers = [httparse::EMPTY_HEADER; 16]; // Adjust the size as needed
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

            for header in req.headers.iter() {
                let name = header.name;
                let value = std::str::from_utf8(header.value).ok()?;
                builder = builder.header(name, value);
            }

            builder.body(()).ok()
        }
        Ok(Status::Partial) => None, // Incomplete request
        Err(_) => None,              // Parsing failed
    }
}

#[cfg(test)]
mod tests {
    use crate::only_in_debug;
    use crate::request::parse_request;
    use std::error::Error;

    #[test]
    fn test_simple() -> Result<(), Box<dyn Error>> {
        only_in_debug();

        let request_str = r#"GET / HTTP/1.1
Host: example.com
User-Agent: curl/7.68.0
Accept: */*
"#;

        let req = parse_request(request_str);
        println!("{:#?}", req);

        Ok(())
    }
}