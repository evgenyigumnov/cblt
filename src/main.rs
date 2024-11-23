use crate::config::{build_config, Directive};
use crate::error::CbltError;
use crate::server::{Server, ServerWorker};
use clap::Parser;
use kdl::KdlDocument;
use log::{debug, error, info};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::Path;
use std::str;
use std::sync::Arc;
use tokio::fs;
use tokio::runtime::Builder;
#[cfg(feature = "trace")]
use tracing::instrument;
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::FmtSubscriber;
mod config;
mod directive;
mod error;
mod file_server;
mod request;
mod response;
mod reverse_proxy;
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

    /// Enable reload feature
    #[arg(long)]
    reload: bool,
}

fn main() -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    only_in_debug();
    #[cfg(not(debug_assertions))]
    only_in_production();
    let num_cpus = std::thread::available_parallelism()?.get();
    let runtime = Builder::new_multi_thread()
        .worker_threads(num_cpus)
        .enable_all()
        .build()?;

    runtime.block_on(async {
        server(num_cpus).await?;
        Ok(())
    })
}
#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
async fn server(num_cpus: usize) -> anyhow::Result<()> {
    let args = Arc::new(Args::parse());

    if args.reload {
        let reload_file_path = Path::new("reload");
        if reload_file_path.exists() {
            anyhow::bail!("File 'reload' already exists");
        } else {
            std::fs::File::create(reload_file_path)?;
            info!("Reloading 'Cbltfile'  has been initiated");
        }
        return Ok(());
    }
    info!("Workers amount: {}", num_cpus);

    let max_connections: usize = args.max_connections;
    info!("Max connections: {}", max_connections);

    let servers: HashMap<u16, Server> = load_servers_from_config(args.clone()).await?;

    debug!("{:#?}", servers);
    use tokio::sync::watch;

    let (tx, mut rx) = watch::channel(servers);

    let args_clone = args.clone();
    tokio::spawn(async move {
        let mut sever_supervisor = ServerSupervisor {
            workers: HashMap::new(),
        };

        loop {
            {
                let servers = rx.borrow_and_update().clone();
                if let Err(err) = &sever_supervisor
                    .process_workers(args_clone.clone(), servers)
                    .await
                {
                    error!("Error: {}", err);
                    std::process::exit(0);
                }
            }

            if rx.changed().await.is_err() {
                break;
            }
        }
    });

    let args = args.clone();

    tokio::spawn(async move {
        let reload_file_path = Path::new("reload");

        loop {
            if reload_file_path.exists() {
                match load_servers_from_config(args.clone()).await {
                    Ok(servers) => {
                        if let Err(err) = tx.send(servers) {
                            error!("Error: {}", err);
                        }
                        if let Err(err) = std::fs::remove_file(reload_file_path) {
                            error!("Error: {}", err);
                        }
                    }
                    Err(err) => {
                        error!("Error: {}", err);
                    }
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });

    info!("CBLT started");
    tokio::signal::ctrl_c().await?;
    info!("CBLT stopped");

    Ok(())
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
async fn load_servers_from_config(args: Arc<Args>) -> Result<HashMap<u16, Server>, CbltError> {
    let cbltfile_content = fs::read_to_string(&args.cfg).await?;
    let doc: KdlDocument = cbltfile_content.parse()?;
    let config = build_config(&doc)?;

    build_servers(config)
}

pub struct ServerSupervisor {
    workers: HashMap<u16, ServerWorker>,
}

impl ServerSupervisor {
    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    async fn process_workers(
        &mut self,
        args: Arc<Args>,
        servers: HashMap<u16, Server>,
    ) -> Result<(), CbltError> {
        let for_stop: Vec<u16> = self
            .workers
            .keys()
            .filter(|port| !servers.contains_key(port)).copied()
            .collect();
        for port in for_stop {
            if let Some(worker) = self.workers.remove(&port) {
                worker
                    .is_running
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                worker.notify_stop.notify_one();
                info!("Server worker stopped on port: {}", port);
            }
        }

        for (port, server) in servers {
            if let Some(worker) = self.workers.get_mut(&port) {
                worker.update(server.hosts, server.cert, server.key).await?;
                info!("Server worker updated on port: {}", port);
            } else if let Ok(server_worker) = ServerWorker::new(server.clone()).await {
                if let Err(err) = server_worker.run(args.max_connections).await {
                    error!("Error: {}", err);
                }
                self.workers.insert(port, server_worker);
            } else {
                error!("Error creating server worker");
            }
        }

        Ok(())
    }
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn build_servers(
    config: HashMap<String, Vec<Directive>>,
) -> Result<HashMap<u16, Server>, CbltError> {
    let mut servers: HashMap<u16, Server> = HashMap::new(); // Port -> Server

    for (host, directives) in config {
        let mut port = 80;
        let mut cert_path = None;
        let mut key_path = None;
        directives.iter().for_each(|d| {
            if let Directive::TlS { cert, key } = d {
                port = 443;
                cert_path = Some(cert.to_string());
                key_path = Some(key.to_string());
            }
        });
        let parsed_host = ParsedHost::from_str(&host);
        let port = parsed_host.port.unwrap_or(port);
        debug!("Host: {}, Port: {}", host, port);
        let cert_path = cert_path;

        let key_path = key_path;

        match servers.entry(port) {
            Entry::Occupied(mut server) => {
                let hosts = &mut server.get_mut().hosts;
                hosts.insert(host, directives);
                server.get_mut().cert = cert_path.clone();
                server.get_mut().key = key_path.clone();
            }
            Entry::Vacant(new_server) => {
                let mut hosts = HashMap::new();
                let host = parsed_host.host;
                hosts.insert(host, directives);

                new_server.insert(Server {
                    port,
                    hosts,
                    cert: cert_path.clone(),
                    key: key_path.clone(),
                });
            }
        }
    }
    Ok(servers)
}

#[allow(dead_code)]
pub fn only_in_debug() {
    let _ =
        env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("debug")).try_init();
}

#[allow(dead_code)]
fn only_in_production() {
    let _ =
        env_logger::Builder::from_env(env_logger::Env::new().default_filter_or("info")).try_init();
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::TRACE) // Set the maximum log level
        .with_span_events(FmtSpan::CLOSE)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set subscriber");
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn matches_pattern(pattern: &str, path: &str) -> bool {
    if pattern == "*" {
        true
    } else if let Some(prefix) = pattern.strip_suffix("*") {
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
    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
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
