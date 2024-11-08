use http::{Request};
use tracing::instrument;

#[instrument]
pub fn parse_request(req_str: &str) -> Option<Request<()>> {
    let mut lines = req_str.lines();

    // Parse the request line
    let mut request_line_parts = lines.next()?.split_whitespace();
    let method = request_line_parts.next()?;
    let uri = request_line_parts.next()?;
    let version_str = request_line_parts.next()?;
    if request_line_parts.next().is_some() {
        return None;
    }

    let version = match version_str {
        "HTTP/1.1" => http::Version::HTTP_11,
        "HTTP/1.0" => http::Version::HTTP_10,
        _ => return None,
    };

    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .version(version);

    // Parse headers
    for line in lines {
        if line.is_empty() {
            break;
        }
        let mut header_parts = line.splitn(2, ": ");
        let key = header_parts.next()?;
        let value = header_parts.next()?;
        builder = builder.header(key, value);
    }

    builder.body(()).ok()
}


#[cfg(test)]
mod tests {
    use std::error::Error;
    use crate::request::parse_request;

    #[test]
    fn test_simple() ->  Result<(), Box<dyn Error>> {

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