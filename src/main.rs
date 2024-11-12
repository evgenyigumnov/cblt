use crate::buffer_pool::{BufferPool, SmartVector};
use crate::config::{build_config, Directive};
use crate::request::{socket_to_request, BUF_SIZE};
use crate::response::{error_response, log_request_response, send_response};
use clap::Parser;
use http::{Response, StatusCode};
use kdl::KdlDocument;
use log::{debug, error, info};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::HashMap;
use std::error::Error;
use std::str;
use std::sync::Arc;
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::Semaphore;
use tokio_rustls::{rustls, TlsAcceptor};
use tracing::instrument;
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::FmtSubscriber;

mod config;
mod request;
mod response;

mod file_server;
mod reverse_proxy;

mod buffer_pool;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Configuration file path
    #[arg(long, default_value = "./Cbltfile")]
    cfg: String,

    /// Maximum number of connections
    #[arg(long, default_value_t = 10000)]
    max_connections: usize,
}

#[derive(Debug, Clone)]
pub struct Server {
    pub port: u16,
    pub hosts: HashMap<String, Vec<Directive>>, // Host -> Directives
    pub cert: Option<String>,
    pub key: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    #[cfg(debug_assertions)]
    only_in_debug();
    #[cfg(not(debug_assertions))]
    only_in_production();
    let num_cpus = num_cpus::get();
    info!("Workers amount: {}", num_cpus);
    let runtime = Builder::new_multi_thread()
        .worker_threads(num_cpus)
        .enable_all()
        .build()?;

    runtime.block_on(async {
        server().await?;
        Ok(())
    })
}
async fn server() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let max_connections: usize = args.max_connections;
    info!("Max connections: {}", max_connections);

    let cbltfile_content = match fs::read_to_string(&args.cfg).await {
        Ok(file) => file,
        Err(_) => {
            error!("Cbltfile not found");
            panic!("Cbltfile not found");
        }
    };
    let doc: KdlDocument = cbltfile_content.parse()?;
    let config = build_config(&doc)?;

    let mut servers: HashMap<u16, Server> = HashMap::new(); // Port -> Server

    for (host, directives) in config {
        let mut port = 80;
        let mut cert_path = None;
        let mut key_path = None;
        directives.iter().for_each(|d| {
            if let Directive::Tls { cert, key } = d {
                port = 443;
                cert_path = Some(cert.to_string());
                key_path = Some(key.to_string());
            }
        });
        if host.contains(":") {
            let parts: Vec<&str> = host.split(":").collect();
            port = parts[1].parse().unwrap();
        }
        debug!("Host: {}, Port: {}", host, port);
        servers
            .entry(port)
            .and_modify(|s| {
                let hosts = &mut s.hosts;
                hosts.insert(host.to_string(), directives.clone());
                s.cert = cert_path.clone();
                s.key = key_path.clone();
            })
            .or_insert({
                let mut hosts = HashMap::new();
                let host = if host.contains(":") {
                    host.split(":").collect::<Vec<&str>>()[0]
                } else {
                    host.as_str()
                };
                hosts.insert(host.to_string(), directives.clone());
                Server {
                    port,
                    hosts,
                    cert: cert_path,
                    key: key_path,
                }
            });
    }

    debug!("{:#?}", servers);

    for (_, server) in servers {
        tokio::spawn(async move {
            match server_task(&server, max_connections).await {
                Ok(_) => {}
                Err(err) => {
                    error!("Error: {}", err);
                }
            }
        });
    }
    info!("Cblt started");
    tokio::signal::ctrl_c().await?;
    info!("Cblt stopped");

    Ok(())
}

async fn server_task(server: &Server, max_connections: usize) -> Result<(), Box<dyn Error>> {
    let acceptor = if server.cert.is_some() {
        let certs = CertificateDer::pem_file_iter(server.cert.clone().unwrap())?
            .collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(server.key.clone().unwrap())?;
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        Some(TlsAcceptor::from(Arc::new(server_config)))
    } else {
        None
    };

    let semaphore = Arc::new(Semaphore::new(max_connections));
    let addr = format!("0.0.0.0:{}", server.port);
    let listener = TcpListener::bind(addr).await?;
    let buffer_pool = Arc::new(BufferPool::new(max_connections, BUF_SIZE));
    info!("Listen port: {}", server.port);
    loop {
        let buffer_pool_arc = buffer_pool.clone();
        let acceptor_clone = acceptor.clone();
        let server_clone = server.clone();
        let (mut stream, _) = listener.accept().await?;
        let permit = semaphore.clone().acquire_owned().await?;
        tokio::spawn(async move {
            let _permit = permit;
            let buffer = buffer_pool_arc.get_buffer().await.unwrap();
            match acceptor_clone {
                None => {
                    if let Err(err) =
                        directive_process(&mut stream, &server_clone, buffer.clone()).await
                    {
                        error!("Error: {}", err);
                    }
                }
                Some(ref acceptor) => match acceptor.accept(stream).await {
                    Ok(mut stream) => {
                        if let Err(err) =
                            directive_process(&mut stream, &server_clone, buffer.clone()).await
                        {
                            error!("Error: {}", err);
                        }
                    }
                    Err(err) => {
                        error!("Error: {}", err);
                    }
                },
            }
            buffer.lock().await.clear();
            buffer_pool_arc.return_buffer(buffer).await;
        });
    }
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
async fn directive_process<S>(
    socket: &mut S,
    server: &Server,
    buffer: SmartVector,
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
            let req_ref = &request;
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
                        let ret =
                            file_server::file_directive(root_path, &request, socket, req_ref).await;
                        match ret {
                            Ok(_) => {
                                log_request_response::<Vec<u8>>(
                                    req_ref.method(),
                                    req_ref.uri(),
                                    req_ref.headers(),
                                    StatusCode::OK,
                                );
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
                                            log_request_response::<Vec<u8>>(
                                                req_ref.method(),
                                                req_ref.uri(),
                                                req_ref.headers(),
                                                status_code,
                                            );
                                            return Ok(());
                                        }
                                        Err(err) => {
                                            log_request_response::<Vec<u8>>(
                                                req_ref.method(),
                                                req_ref.uri(),
                                                req_ref.headers(),
                                                StatusCode::INTERNAL_SERVER_ERROR,
                                            );
                                            return Err(err);
                                        }
                                    }
                                }
                                CbltError::DirectiveNotMatched => {}
                                err => {
                                    log_request_response::<Vec<u8>>(
                                        req_ref.method(),
                                        req_ref.uri(),
                                        req_ref.headers(),
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
                            req_ref,
                            pattern,
                            destination,
                        )
                        .await
                        {
                            Ok(status) => {
                                log_request_response::<Vec<u8>>(
                                    req_ref.method(),
                                    req_ref.uri(),
                                    req_ref.headers(),
                                    status,
                                );
                                return Ok(());
                            }
                            Err(err) => match err {
                                CbltError::RequestError {
                                    details: _,
                                    status_code,
                                } => {
                                    log_request_response::<Vec<u8>>(
                                        req_ref.method(),
                                        req_ref.uri(),
                                        req_ref.headers(),
                                        status_code,
                                    );
                                    return Ok(());
                                }
                                CbltError::DirectiveNotMatched => {}
                                CbltError::ResponseError {
                                    details: _,
                                    status_code,
                                } => {
                                    log_request_response::<Vec<u8>>(
                                        req_ref.method(),
                                        req_ref.uri(),
                                        req_ref.headers(),
                                        status_code,
                                    );
                                    return Ok(());
                                }
                                other => {
                                    log_request_response::<Vec<u8>>(
                                        req_ref.method(),
                                        req_ref.uri(),
                                        req_ref.headers(),
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
                            .body(Vec::new()) // Empty body for redirects
                            .unwrap();
                        match send_response(socket, response).await {
                            Ok(_) => {
                                log_request_response::<Vec<u8>>(
                                    req_ref.method(),
                                    req_ref.uri(),
                                    req_ref.headers(),
                                    StatusCode::FOUND,
                                );
                                return Ok(());
                            }
                            Err(err) => {
                                log_request_response::<Vec<u8>>(
                                    req_ref.method(),
                                    req_ref.uri(),
                                    req_ref.headers(),
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
                log_request_response::<Vec<u8>>(
                    req_ref.method(),
                    req_ref.uri(),
                    req_ref.headers(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                );
                return Err(err);
            }
            log_request_response::<Vec<u8>>(
                req_ref.method(),
                req_ref.uri(),
                req_ref.headers(),
                StatusCode::NOT_FOUND,
            );
            Ok(())
        }
    }
}

#[allow(dead_code)]
pub fn only_in_debug() {
    let _ =
        env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("debug")).try_init();
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::TRACE) // Set the maximum log level
        .with_span_events(FmtSpan::CLOSE)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set subscriber");
}

#[allow(dead_code)]
fn only_in_production() {
    let _ =
        env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("info")).try_init();
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
fn matches_pattern(pattern: &str, path: &str) -> bool {
    if pattern == "*" {
        true
    } else if pattern.ends_with("*") {
        let prefix = &pattern[..pattern.len() - 1];
        path.starts_with(prefix)
    } else {
        pattern == path
    }
}

#[derive(Error, Debug)]
pub enum CbltError {
    #[error("ParseRequestError: {details:?}")]
    ParseRequestError { details: String },
    #[error("RequestError: {details:?}")]
    RequestError {
        details: String,
        status_code: StatusCode,
    },
    #[error("DirectiveNotMatched")]
    DirectiveNotMatched,
    #[error("ResponseError: {details:?}")]
    ResponseError {
        details: String,
        status_code: StatusCode,
    },
    #[error("IOError: {source:?}")]
    IOError {
        #[from]
        source: std::io::Error,
    },
    // from reqwest::Error
    #[error("ReqwestError: {source:?}")]
    ReqwestError {
        #[from]
        source: reqwest::Error,
    },
}

#[derive(Debug)]
pub struct CbltRequest {
    pub host: String,
    pub port: u16,
    pub uri: String,
    pub method: String,
    pub status_code: StatusCode,
}
