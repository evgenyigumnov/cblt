use crate::buffer_pool::{BufferPool, SmartVector};
use crate::config::{build_config, Directive};
use crate::error::CbltError;
use crate::request::{socket_to_request, BUF_SIZE};
use crate::response::{error_response, log_request_response, send_response};
use anyhow::Context;
use clap::Parser;
use http::{Response, StatusCode};
use kdl::KdlDocument;
use log::{debug, error, info};
use reqwest::Client;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::HashMap;
use std::str;
use std::sync::Arc;
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

mod error;

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

fn main() -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    only_in_debug();
    #[cfg(not(debug_assertions))]
    only_in_production();
    let num_cpus = std::thread::available_parallelism()?.get();
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
async fn server() -> anyhow::Result<()> {
    let args = Args::parse();
    let max_connections: usize = args.max_connections;
    info!("Max connections: {}", max_connections);

    let cbltfile_content = fs::read_to_string(&args.cfg)
        .await
        .context("Failed to read Cbltfile")?;
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
        let parsed_host = ParsedHost::from_str(&host);
        let port = parsed_host.port.unwrap_or(port);
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
                let host = parsed_host.host;
                hosts.insert(host, directives.clone());
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
    info!("CBLT started");
    tokio::signal::ctrl_c().await?;
    info!("CBLT stopped");

    Ok(())
}

async fn server_task(server: &Server, max_connections: usize) -> Result<(), CbltError> {
    let acceptor = if server.cert.is_some() {
        let certs =
            CertificateDer::pem_file_iter(server.cert.clone().ok_or(CbltError::AbsentCert)?)?
                .collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(server.key.clone().ok_or(CbltError::AbsentKey)?)?;
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        Some(TlsAcceptor::from(Arc::new(server_config)))
    } else {
        None
    };

    let semaphore = Arc::new(Semaphore::new(max_connections));
    let port_string = server.port.to_string();
    let port_str = port_string.as_str();
    let addr = ["0.0.0.0:", port_str].concat();
    let listener = TcpListener::bind(addr).await?;
    let buffer_pool = Arc::new(BufferPool::new(max_connections, BUF_SIZE));
    info!("Listen port: {}", server.port);
    let client_reqwest = reqwest::Client::new();
    loop {
        let client_reqwest = client_reqwest.clone();
        let buffer_pool_arc = buffer_pool.clone();
        let acceptor_clone = acceptor.clone();
        let server_clone = server.clone();
        let (mut stream, _) = listener.accept().await?;
        let permit = semaphore.clone().acquire_owned().await?;
        tokio::spawn(async move {
            let _permit = permit;
            let buffer = buffer_pool_arc.get_buffer().await;
            match acceptor_clone {
                None => {
                    if let Err(err) = directive_process(
                        &mut stream,
                        &server_clone,
                        buffer.clone(),
                        client_reqwest.clone(),
                    )
                    .await
                    {
                        error!("Error: {}", err);
                    }
                }
                Some(ref acceptor) => match acceptor.accept(stream).await {
                    Ok(mut stream) => {
                        if let Err(err) = directive_process(
                            &mut stream,
                            &server_clone,
                            buffer.clone(),
                            client_reqwest.clone(),
                        )
                        .await
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

pub struct ParsedHost {
    pub host: String,
    pub port: Option<u16>,
}

impl ParsedHost {
    fn from_str(host_str: &str) -> Self {
        if let Some((host_part, port_part)) = host_str.split_once(':') {
            let port = port_part.parse().ok();
            ParsedHost {
                host: host_part.to_string(),
                port,
            }
        } else {
            ParsedHost {
                host: host_str.to_string(),
                port: None,
            }
        }
    }
}
