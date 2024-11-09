use std::collections::HashMap;
use crate::config::{build_config, Directive};
use crate::request::{socket_to_request};
use crate::response::{error_response, send_response};
use http::{Response, StatusCode};
use kdl::KdlDocument;
use log::{debug, error, info};
use std::error::Error;
use std::str;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{ AsyncReadExt};
use tokio::net::TcpListener;
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::FmtSubscriber;
use tracing::instrument;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncWriteExt};
use tokio_rustls::{rustls, TlsAcceptor};

mod config;
mod request;
mod response;

mod file_server;
mod reverse_proxy;

#[derive(Debug)]
pub struct Server {
    pub port: u16,
    pub hosts: HashMap<String, Vec<Directive>>, // Host -> Directives
    pub cert: Option<String>,
    pub key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    info!("Cblt started");
    #[cfg(debug_assertions)]
    only_in_debug();
    #[cfg(not(debug_assertions))]
    only_in_production();
    // Read configuration from Cbltfile
    let cbltfile_content = fs::read_to_string("Cbltfile").await?;
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
        servers.entry(port).and_modify(
            |s| {
                let hosts = &mut s.hosts;
                hosts.insert(host.to_string(), directives.clone());
                s.cert = cert_path.clone();
                s.key = key_path.clone();
            },
        ).or_insert({
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
            match server_task(&server).await {
                Ok(_) => {}
                Err(err) => {
                    error!("Error: {}", err);
                }
            }
        });
    }

    tokio::signal::ctrl_c().await?;

    Ok(())
}

async fn server_task(server: &Server) -> Result<(), Box<dyn Error>> {
        let acceptor = if server.cert.is_some() {
            let certs = CertificateDer::pem_file_iter(server.cert.clone().unwrap())?.collect::<Result<Vec<_>, _>>()?;
            let key = PrivateKeyDer::from_pem_file(server.key.clone().unwrap())?;
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)?;
            Some(TlsAcceptor::from(Arc::new(server_config)))
        } else {
            None
        };

        let addr = format!("0.0.0.0:{}", server.port);
        let listener = TcpListener::bind(addr).await?;

        loop {
            let (mut stream, _) = listener.accept().await?;
            match acceptor {
                None => {
                    directive_process(&mut stream, &server).await;
                }
                Some(ref acceptor) => {
                    match acceptor.accept(stream).await {
                        Ok(mut stream) => {
                            directive_process(&mut stream, &server).await;
                        }
                        Err(err) => {
                            error!("Error: {}", err);
                        }
                    }

                }
            }
        }
}

#[cfg_attr(debug_assertions, instrument(level = "trace", skip_all))]
async fn directive_process<S>(socket: &mut S, server: &Server)
    where S: AsyncReadExt + AsyncWriteExt + Unpin
{
    match socket_to_request(socket).await {
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
            let cfg_opt  = server.hosts.iter().find(|(k, _)| k.starts_with("*"));
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
                Some((_, cfg)) => cfg
            };

            let mut root_path = None;
            let mut handled = false;

            for directive in host_config {
                match directive {
                    Directive::Root { pattern, path } => {
                        #[cfg(debug_assertions)]
                        debug!("Root: {} -> {}", pattern, path);
                        if matches_pattern(pattern, request.uri().path()) {
                            root_path = Some(path.clone());
                        }
                    }
                    Directive::FileServer => {
                        #[cfg(debug_assertions)]
                        debug!("File server");
                        file_server::directive(&root_path, &request, &mut handled, socket, req_opt).await;
                        break;
                    }
                    Directive::ReverseProxy {
                        pattern,
                        destination,
                    } => {
                        #[cfg(debug_assertions)]
                        debug!("Reverse proxy: {} -> {}", pattern, destination);
                        reverse_proxy::directive(&request, &mut handled, socket, req_opt, pattern, destination).await;
                        break;
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
