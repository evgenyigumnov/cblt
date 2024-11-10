use crate::matches_pattern;
use crate::response::{error_response, log_request_response, send_response};
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use log::debug;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn directive<S>(
    request: &Request<Vec<u8>>,
    handled: &mut bool,
    socket: &mut S,
    req_opt: Option<&Request<Vec<u8>>>,
    pattern: &String,
    destination: &String,
) where
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
                let headers = resp.headers().clone();
                let body = resp.bytes().await.unwrap_or_else(|_| Bytes::new());

                let mut response_builder = Response::builder().status(status);

                for (key, value) in headers.iter() {
                    response_builder = response_builder.header(key, value);
                }

                let response = response_builder.body(body.to_vec()).unwrap();
                log_request_response(req_opt, &response);
                let _ = send_response(socket, response, req_opt).await;
                *handled = true;

                return;
            }
            Err(_) => {
                let response = error_response(StatusCode::BAD_GATEWAY);
                log_request_response(req_opt, &response);
                let _ = send_response(socket, response, req_opt).await;
                *handled = true;
                return;
            }
        }
    }
}
