use crate::config::{build_config, Directive};
use crate::error::CbltError;
use crate::server::{server_init, Server};
use anyhow::Context;
use clap::Parser;
use kdl::KdlDocument;
use log::{debug, error, info};
use std::collections::HashMap;
use std::str;
use tokio::fs;
use tokio::runtime::Builder;
use tracing::instrument;
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::FmtSubscriber;

mod config;
mod request;
mod response;

mod directive;

mod error;

mod file_server;
mod reverse_proxy;

mod buffer_pool;

mod server;

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
            match server_init(&server, max_connections).await {
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
