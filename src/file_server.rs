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
    fallback_file: Option<&str>, // fallback file path
    request: &Request<BytesMut>,
    socket: &mut S,
) -> Result<StatusCode, CbltError>
where
    S: AsyncWrite + Unpin,
{
    match root_path {
        None => Err(CbltError::ResponseError {
            details: "".to_string(),
            status_code: StatusCode::INTERNAL_SERVER_ERROR,
        }),
        Some(root) => {
            if let Some(mut file_path) = sanitize_path(
                Path::new(root),
                request.uri().path().trim_start_matches('/'),
            ) {
                if file_path.is_dir() {
                    file_path.push("index.html");
                }

                // try to open the requested file
                let file_result = File::open(&file_path).await;

                let (file, final_path) = match file_result {
                    Ok(file) => (file, file_path),
                    Err(_) => {
                        // if it fails, check for the fallback file
                        if let Some(fallback) = fallback_file {
                            let fallback_path =
                                Path::new(root).join(fallback.trim_start_matches('/'));
                            match File::open(&fallback_path).await {
                                Ok(fallback_file) => (fallback_file, fallback_path),
                                Err(err) => {
                                    return Err(CbltError::ResponseError {
                                        details: format!(
                                            "Neither requested file nor fallback file found: {}",
                                            err
                                        ),
                                        status_code: StatusCode::NOT_FOUND,
                                    })
                                }
                            }
                        } else {
                            return Err(CbltError::ResponseError {
                                details: "File not found".to_string(),
                                status_code: StatusCode::NOT_FOUND,
                            });
                        }
                    }
                };

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
                        ranged_file_response(file, &final_path, content_length, range).await?;
                    send_response_file(socket, response, request).await?;
                    Ok(StatusCode::PARTIAL_CONTENT)
                } else {
                    let response = file_response(file, &final_path, content_length)?;
                    send_response_file(socket, response, request).await?;
                    Ok(StatusCode::OK)
                }
            } else {
                Err(CbltError::DirectiveNotMatched)
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
fn file_response(
    file: File,
    file_path: &PathBuf,
    content_length: u64,
) -> Result<Response<File>, CbltError> {
    // Guess the MIME type based on the file extension
    let mime_type = mime_guess::from_path(file_path)
        .first_or_octet_stream()
        .to_string();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Length", content_length)
        .header("Content-Type", mime_type)
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
