use crate::response::{error_response, send_response, send_response_file};
use http::{Request, Response, StatusCode};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::instrument;

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn directive<S>(
    root_path: &Option<String>,
    request: &Request<()>,
    handled: &mut bool,
    socket: &mut S,
    req_opt: Option<&Request<()>>,
) where
    S: AsyncWriteExt + Unpin,
{
    if let Some(root) = root_path {
        let mut file_path = PathBuf::from(root);
        file_path.push(request.uri().path().trim_start_matches('/'));

        if file_path.is_dir() {
            file_path.push("index.html");
        }

        match File::open(&file_path).await {
            Ok(file) => {
                let content_length = file_size(&file).await;
                let response = file_response(file, content_length);
                let _ = send_response_file(socket, response, req_opt).await;
                *handled = true;
                return;
            }
            Err(_) => {
                let response = error_response(StatusCode::NOT_FOUND);
                let _ = send_response(&mut *socket, response, req_opt).await;
                *handled = true;
                return;
            }
        }
    } else {
        let response = error_response(StatusCode::INTERNAL_SERVER_ERROR);
        let _ = send_response(&mut *socket, response, req_opt).await;
        *handled = true;
        return;
    }
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
async fn file_size(file: &File) -> u64 {
    let metadata = file.metadata().await.unwrap();
    metadata.len()
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
fn file_response(file: File, content_length: u64) -> Response<File> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Length", content_length)
        .body(file)
        .unwrap()
}
