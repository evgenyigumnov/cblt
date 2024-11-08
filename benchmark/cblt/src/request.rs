use http::Request;
use tracing::instrument;

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

use http::Version;
use httparse::Status;

#[instrument(level = "trace", skip_all)]
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
