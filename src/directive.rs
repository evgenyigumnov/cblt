use crate::buffer_pool::SmartVector;
use crate::config::Directive;
use crate::error::CbltError;
use crate::request::socket_to_request;
use crate::response::{error_response, log_request_response, send_response};
use crate::server::Server;
use crate::{file_server, matches_pattern, reverse_proxy};
use http::{Response, StatusCode};
use log::{debug, info};
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
pub async fn directive_process<S>(
    socket: &mut S,
    server: &Server,
    buffer: SmartVector,
    client_reqwest: Client,
) -> Result<(), CbltError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    match socket_to_request(socket, buffer).await {
        Err(_) => {
            let response = error_response(StatusCode::BAD_REQUEST);
            let ret = send_response(socket, response).await;
            match ret {
                Ok(()) => {}
                Err(err) => {
                    info!("Error: {}", err);
                    return Err(err);
                }
            }
            return Err(CbltError::ParseRequestError {
                details: "Parse request error".to_string(),
            });
        }
        Ok(request) => {
            let host = match request.headers().get("Host") {
                Some(h) => h.to_str().unwrap_or(""),
                None => "",
            };

            // find host starting with "*"
            let cfg_opt = server.hosts.iter().find(|(k, _)| k.starts_with("*"));
            let host_config = match cfg_opt {
                None => {
                    let host_config = match server.hosts.get(host) {
                        Some(cfg) => cfg,
                        None => {
                            let response = error_response(StatusCode::FORBIDDEN);
                            let _ = send_response(socket, response).await;
                            return Err(CbltError::ResponseError {
                                details: "Forbidden".to_string(),
                                status_code: StatusCode::FORBIDDEN,
                            });
                        }
                    };
                    host_config
                }
                Some((_, cfg)) => cfg,
            };

            let mut root_path: Option<&str> = None;

            for directive in host_config {
                match directive {
                    Directive::Root { pattern, path } => {
                        #[cfg(debug_assertions)]
                        debug!("Root: {} -> {}", pattern, path);
                        if matches_pattern(pattern, request.uri().path()) {
                            root_path = Some(path);
                        }
                    }
                    Directive::FileServer => {
                        #[cfg(debug_assertions)]
                        debug!("File server");
                        let ret = file_server::file_directive(root_path, &request, socket).await;
                        match ret {
                            Ok(_) => {
                                log_request_response::<Vec<u8>>(&request, StatusCode::OK);
                                return Ok(());
                            }
                            Err(error) => match error {
                                CbltError::ResponseError {
                                    details: _,
                                    status_code,
                                } => {
                                    let response = error_response(status_code);
                                    match send_response(socket, response).await {
                                        Ok(()) => {
                                            log_request_response::<Vec<u8>>(&request, status_code);
                                            return Ok(());
                                        }
                                        Err(err) => {
                                            log_request_response::<Vec<u8>>(
                                                &request,
                                                StatusCode::INTERNAL_SERVER_ERROR,
                                            );
                                            return Err(err);
                                        }
                                    }
                                }
                                CbltError::DirectiveNotMatched => {}
                                err => {
                                    log_request_response::<Vec<u8>>(
                                        &request,
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                    );
                                    return Err(err);
                                }
                            },
                        }
                        break;
                    }
                    Directive::ReverseProxy {
                        pattern,
                        destination,
                    } => {
                        #[cfg(debug_assertions)]
                        debug!("Reverse proxy: {} -> {}", pattern, destination);
                        match reverse_proxy::proxy_directive(
                            &request,
                            socket,
                            pattern,
                            destination,
                            client_reqwest.clone(),
                        )
                        .await
                        {
                            Ok(status) => {
                                log_request_response::<Vec<u8>>(&request, status);
                                return Ok(());
                            }
                            Err(err) => match err {
                                CbltError::DirectiveNotMatched => {}
                                CbltError::ResponseError {
                                    details: _,
                                    status_code,
                                } => {
                                    let response = error_response(status_code);
                                    match send_response(socket, response).await {
                                        Ok(()) => {
                                            log_request_response::<Vec<u8>>(&request, status_code);
                                            return Ok(());
                                        }
                                        Err(err) => {
                                            log_request_response::<Vec<u8>>(
                                                &request,
                                                StatusCode::INTERNAL_SERVER_ERROR,
                                            );
                                            return Err(err);
                                        }
                                    }
                                }
                                other => {
                                    log_request_response::<Vec<u8>>(
                                        &request,
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                    );
                                    return Err(other);
                                }
                            },
                        }
                    }
                    Directive::Redir { destination } => {
                        let dest = destination.replace("{uri}", request.uri().path());
                        let response = Response::builder()
                            .status(StatusCode::FOUND)
                            .header("Location", &dest)
                            .body(Vec::new())?; // Empty body for redirects?
                        match send_response(socket, response).await {
                            Ok(_) => {
                                log_request_response::<Vec<u8>>(&request, StatusCode::FOUND);
                                return Ok(());
                            }
                            Err(err) => {
                                log_request_response::<Vec<u8>>(
                                    &request,
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                );
                                return Err(err);
                            }
                        }
                    }
                    Directive::Tls { .. } => {}
                }
            }

            let response = error_response(StatusCode::NOT_FOUND);
            if let Err(err) = send_response(socket, response).await {
                log_request_response::<Vec<u8>>(&request, StatusCode::INTERNAL_SERVER_ERROR);
                return Err(err);
            }
            log_request_response::<Vec<u8>>(&request, StatusCode::NOT_FOUND);
            Ok(())
        }
    }
}
