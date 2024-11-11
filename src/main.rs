use crate::buffer_pool::{BufferPool, SmartVector};
use crate::config::{build_config, Directive};
use crate::request::{socket_to_request, BUF_SIZE};
use crate::response::{error_response, send_response};
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
    // #[cfg(debug_assertions)]
    // only_in_debug();
    // #[cfg(not(debug_assertions))]
    // only_in_production();
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
                    directive_process(&mut stream, &server_clone, buffer.clone()).await;
                }
                Some(ref acceptor) => match acceptor.accept(stream).await {
                    Ok(mut stream) => {
                        directive_process(&mut stream, &server_clone, buffer.clone()).await;
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
async fn directive_process<S>(socket: &mut S, server: &Server, buffer: SmartVector)
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    match socket_to_request(socket, buffer).await {
        None => {
            return;
        }
        Some(request) => {
            let req_opt = Some(&request);
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
                            let req_opt = Some(&request);
                            let response = error_response(StatusCode::FORBIDDEN);
                            let _ = send_response(socket, response, req_opt).await;
                            return;
                        }
                    };
                    host_config
                }
                Some((_, cfg)) => cfg,
            };

            let mut root_path:Option<&str> = None;
            let mut handled = false;

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
                        file_server::file_directive(
                            root_path,
                            &request,
                            &mut handled,
                            socket,
                            req_opt,
                        )
                        .await;
                        break;
                    }
                    Directive::ReverseProxy {
                        pattern,
                        destination,
                    } => {
                        #[cfg(debug_assertions)]
                        debug!("Reverse proxy: {} -> {}", pattern, destination);
                        reverse_proxy::proxy_directive(
                            &request,
                            &mut handled,
                            socket,
                            req_opt,
                            pattern,
                            destination,
                        )
                        .await;
                        if handled {
                            break;
                        }
                    }
                    Directive::Redir { destination } => {
                        let dest = destination.replace("{uri}", request.uri().path());
                        let response = Response::builder()
                            .status(StatusCode::FOUND)
                            .header("Location", &dest)
                            .body(Vec::new()) // Empty body for redirects
                            .unwrap();
                        let _ = send_response(socket, response, req_opt).await;
                        handled = true;
                        break;
                    }
                    Directive::Tls { .. } => {}
                }
            }

            if !handled {
                let response = error_response(StatusCode::NOT_FOUND);
                let _ = send_response(socket, response, req_opt).await;
            }
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
