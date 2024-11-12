use crate::response::send_response_stream;
use crate::{matches_pattern, CBLTError};
use http::{Request, Response, StatusCode};
use log::debug;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn proxy_directive<S>(
    request: &Request<Vec<u8>>,
    socket: &mut S,
    req_ref: &Request<Vec<u8>>,
    pattern: &String,
    destination: &String,
) -> Result<StatusCode, CBLTError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    if matches_pattern(pattern, request.uri().path()) {
        let dest_uri = format!("{}{}", destination, request.uri().path());
        #[cfg(debug_assertions)]
        debug!("Destination URI: {}", dest_uri);
        let client = reqwest::Client::new();
        let mut req_builder = client.request(request.method().clone(), &dest_uri);

        for (key, value) in request.headers().iter() {
            req_builder = req_builder.header(key, value);
        }
        let body = request.body();
        if !body.is_empty() {
            req_builder = req_builder.body(body.clone());
        }

        match req_builder.send().await {
            Ok(resp) => {
                let status = resp.status();
                let headers = resp.headers();
                let mut response_builder = Response::builder().status(status);
                for (key, value) in headers.iter() {
                    response_builder = response_builder.header(key, value);
                }
                let mut stream = resp.bytes_stream();
                let response = response_builder.body("").unwrap();
                send_response_stream(socket, response, req_ref, &mut stream).await?;
                if status != StatusCode::OK {
                    return Err(CBLTError::ResponseError {
                        details: "Bad gateway".to_string(),
                        status_code: status,
                    });
                } else {
                    return Ok(status);
                }
            }
            Err(_) => {
                return Err(CBLTError::ResponseError {
                    details: "Bad gateway".to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                });
            }
        }
    } else {
        return Err(CBLTError::DirectiveNotMatched);
    }
}
