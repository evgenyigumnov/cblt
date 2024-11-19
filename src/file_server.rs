use crate::error::CbltError;
use crate::request::parse_range_header;
use crate::response::{ranged_file_response, send_response_file};
use bytes::BytesMut;
use http::header::RANGE;
use http::{Request, Response, StatusCode};
use std::path::{Component, Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWrite;
#[cfg(feature = "trace")]
use tracing::instrument;

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub async fn file_directive<S>(
    root_path: Option<&str>,
    request: &Request<BytesMut>,
    socket: &mut S,
) -> Result<StatusCode, CbltError>
where
    S: AsyncWrite + Unpin,
{
    match root_path {
        None => {
            return Err(CbltError::ResponseError {
                details: "".to_string(),
                status_code: StatusCode::INTERNAL_SERVER_ERROR,
            });
        }
        Some(root) => {
            if let Some(mut file_path) = sanitize_path(
                Path::new(root),
                request.uri().path().trim_start_matches('/'),
            ) {
                if file_path.is_dir() {
                    file_path.push("index.html");
                }

                match File::open(&file_path).await {
                    Ok(file) => {
                        let content_length = file_size(&file).await?;

                        if let Some(range_header) = request.headers().get(RANGE) {
                            let range_str =
                                range_header
                                    .to_str()
                                    .map_err(|_| CbltError::ResponseError {
                                        details: "Invalid Range header".to_string(),
                                        status_code: StatusCode::BAD_REQUEST,
                                    })?;

                            let range = parse_range_header(range_str, content_length)?;

                            let response =
                                ranged_file_response(file, content_length, range).await?;
                            send_response_file(socket, response, request).await?;
                            return Ok(StatusCode::PARTIAL_CONTENT);
                        } else {
                            let response = file_response(file, content_length)?;
                            send_response_file(socket, response, request).await?;
                            return Ok(StatusCode::OK);
                        }
                    }
                    Err(err) => {
                        return Err(CbltError::ResponseError {
                            details: err.to_string(),
                            status_code: StatusCode::NOT_FOUND,
                        });
                    }
                }
            } else {
                return Err(CbltError::DirectiveNotMatched);
            }
        }
    }
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
async fn file_size(file: &File) -> Result<u64, CbltError> {
    let metadata = file.metadata().await?;
    Ok(metadata.len())
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn file_response(file: File, content_length: u64) -> Result<Response<File>, CbltError> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Length", content_length)
        .body(file)?)
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn sanitize_path(base_path: &Path, requested_path: &str) -> Option<PathBuf> {
    let mut full_path = base_path.to_path_buf();
    let requested_path = Path::new(requested_path);

    for component in requested_path.components() {
        match component {
            Component::Normal(segment) => full_path.push(segment),
            Component::RootDir | Component::Prefix(_) => return None,
            Component::ParentDir => {
                if !full_path.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
        }
    }

    if full_path.starts_with(base_path) {
        Some(full_path)
    } else {
        None
    }
}
