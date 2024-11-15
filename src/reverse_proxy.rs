use crate::response::send_response_stream;
use crate::{matches_pattern, CbltError};
use http::{Request, Response, StatusCode};
use log::debug;
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn proxy_directive<S>(
    request: &Request<Vec<u8>>,
    socket: &mut S,
    pattern: &str,
    destination: &str,
    client_reqwest: Client,
) -> Result<StatusCode, CbltError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    if matches_pattern(pattern, request.uri().path()) {
        let dest_uri = [destination, request.uri().path()].concat();
        #[cfg(debug_assertions)]
        debug!("Destination URI: {}", dest_uri);

        let mut req_builder = client_reqwest.request(request.method().clone(), dest_uri);
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
                let response = response_builder.body("")?;
                send_response_stream(socket, response, request, &mut stream).await?;
                if status != StatusCode::OK {
                    return Err(CbltError::ResponseError {
                        details: "Bad gateway".to_string(),
                        status_code: status,
                    });
                } else {
                    return Ok(status);
                }
            }
            Err(_) => {
                return Err(CbltError::ResponseError {
                    details: "Bad gateway".to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                });
            }
        }
    } else {
        return Err(CbltError::DirectiveNotMatched);
    }
}
