use crate::config::{Directive, LoadBalancePolicy};
use crate::directive::directive_process;
use crate::error::CbltError;
use std::collections::HashMap;

use crate::reverse_proxy::ReverseProxyState;
use humantime::Duration;
use log::{error, info};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use reqwest::ClientBuilder;
use tokio::net::TcpListener;
use tokio::sync::{Notify, RwLock, Semaphore};
use tokio_rustls::TlsAcceptor;
#[cfg(feature = "trace")]
use tracing::instrument;

const REVERSE_PROXY_MAX_IDLE_CONNECTIONS: usize = 100;

#[derive(Debug, Clone)]
pub struct Server {
    pub port: u16,
    pub hosts: HashMap<String, Vec<Directive>>, // Host -> Directives
    pub cert: Option<String>,
    pub key: Option<String>,
}

pub struct ServerWorker {
    pub port: u16,
    pub lock: Arc<SettingsLock>,
    pub is_running: Arc<AtomicBool>,
    pub notify_stop: Arc<Notify>,
}

pub struct SettingsLock {
    settings: RwLock<Arc<ServerSettings>>,
}

impl SettingsLock {
    async fn update(&self, s: Arc<ServerSettings>) {
        for details in self.settings.read().await.hosts.values() {
            for state in details.reverse_proxy_states.values() {
                state.is_running_check.store(false, Ordering::SeqCst);
            }
        }
        let mut settings = self.settings.write().await;
        *settings = s;
    }
    async fn get(&self) -> Arc<ServerSettings> {
        let settings = self.settings.read().await;
        settings.clone()
    }
}

pub struct ServerSettings {
    pub hosts: HashMap<String, HostDetails>,
    pub tls_acceptor: Option<TlsAcceptor>,
}

pub struct HostDetails {
    pub directives: Vec<Directive>,
    pub reverse_proxy_states: HashMap<String, ReverseProxyState>,
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn tls_acceptor_builder(
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<Option<TlsAcceptor>, CbltError> {
    if let (Some(cert_path), Some(key_path)) = (cert_path, key_path) {
        let certs = CertificateDer::pem_file_iter(cert_path)?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(key_path)?;

        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        Ok(Some(TlsAcceptor::from(Arc::new(server_config))))
    } else {
        Ok(None)
    }
}

impl ServerWorker {
    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn new(server: Server) -> Result<Self, CbltError> {
        let tls_acceptor = tls_acceptor_builder(server.cert.as_deref(), server.key.as_deref())?;

        let mut host_details: HashMap<String, HostDetails> = HashMap::new();
        for (k, v) in server.hosts {
            host_details.insert(
                k.to_string(),
                HostDetails {
                    reverse_proxy_states: init_proxy_states(&v).await?,
                    directives: v,
                },
            );
        }

        Ok(ServerWorker {
            port: server.port,
            lock: Arc::new(SettingsLock {
                settings: RwLock::new(
                    ServerSettings {
                        hosts: host_details,
                        tls_acceptor,
                    }
                    .into(),
                ),
            }),
            is_running: Arc::new(AtomicBool::new(true)),
            notify_stop: Arc::new(Notify::new()),
        })
    }

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn run(&self, max_connections: usize) -> Result<(), CbltError> {
        let port = self.port;
        let settings = self.lock.clone();
        let is_running = self.is_running.clone();
        let notify_stop = self.notify_stop.clone();

        tokio::spawn(async move {
            if let Err(err) =
                init_server(port, settings, max_connections, is_running, notify_stop).await
            {
                error!("Error: {}", err);
            }
        });
        Ok(())
    }

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn update(
        &self,
        hosts: HashMap<String, Vec<Directive>>,
        cert_path: Option<String>,
        key_path: Option<String>,
    ) -> Result<(), CbltError> {
        let cert_path_opt = cert_path.as_deref();
        let key_path_opt = key_path.as_deref();
        let tls_acceptor = tls_acceptor_builder(cert_path_opt, key_path_opt)?;
        let mut host_details: HashMap<String, HostDetails> = HashMap::new();
        for (k, v) in hosts {
            host_details.insert(
                k.to_string(),
                HostDetails {
                    reverse_proxy_states: init_proxy_states(&v).await?,
                    directives: v,
                },
            );
        }

        self.lock
            .update(
                ServerSettings {
                    hosts: host_details,
                    tls_acceptor,
                }
                .into(),
            )
            .await;
        Ok(())
    }
}

async fn init_proxy_states(
    directives: &Vec<Directive>,
) -> Result<HashMap<String, ReverseProxyState>, CbltError> {
    let mut reverse_proxy_states: HashMap<String, ReverseProxyState> = HashMap::new(); // (pattern -> ReverseProxyState)
    let client_reqwest = reqwest::Client::new();
    for directive in directives {
        match directive {
            Directive::ReverseProxy {
                pattern,
                destinations,
                options,
            } => {
                let reverse_proxy_state = ReverseProxyState::new(
                    destinations.clone(),
                    options
                        .lb_policy
                        .clone()
                        .unwrap_or(LoadBalancePolicy::RoundRobin),
                    client_reqwest.clone(),
                );

                if let Some(health_uri) = &options.health_uri {
                    let interval = options
                        .health_interval
                        .as_deref()
                        .unwrap_or("10s")
                        .parse::<humantime::Duration>()
                        .unwrap_or(Duration::from(std::time::Duration::from_secs(10)))
                        .as_secs();
                    let timeout = options
                        .health_timeout
                        .as_deref()
                        .unwrap_or("2s")
                        .parse::<humantime::Duration>()
                        .unwrap_or(Duration::from(std::time::Duration::from_secs(2)))
                        .as_secs();
                    reverse_proxy_state
                        .start_health_checks(health_uri.clone(), interval, timeout)
                        .await;

                }
                reverse_proxy_states.insert(pattern.clone(), reverse_proxy_state);
            }
            _ => continue,
        }
    }

    Ok(reverse_proxy_states)
}

async fn init_server(
    port: u16,
    settings_lock: Arc<SettingsLock>,
    max_connections: usize,
    is_running: Arc<AtomicBool>,
    notify_stop: Arc<Notify>,
) -> Result<(), CbltError> {
    let semaphore = Arc::new(Semaphore::new(max_connections));
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on port: {}", port);
    let client_reqwest = reqwest::Client::new();

    while is_running.load(Ordering::SeqCst) {
        let client_reqwest = ClientBuilder::new()
            .pool_max_idle_per_host(REVERSE_PROXY_MAX_IDLE_CONNECTIONS)
            .build()?;
        tokio::select! {
            _ = notify_stop.notified() => {
                break;
            },
            Ok((mut stream, addr)) =  listener.accept() => {
                let permit = semaphore.clone().acquire_owned().await?;
                let settings = settings_lock.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let settings = settings.get().await;
                    let acceptor = settings.tls_acceptor.clone();
                    match acceptor.as_ref() {
                        None => {
                            if let Err(err) = directive_process(
                                &mut stream,
                                settings.clone(),
                                client_reqwest.clone(),
                                addr,
                            )
                            .await
                            {
                                #[cfg(debug_assertions)]
                                error!("Error: {}", err);
                            }
                        }
                        Some(acceptor) => match acceptor.accept(stream).await {
                            Ok(mut stream) => {
                                if let Err(err) = directive_process(
                                    &mut stream,
                                    settings.clone(),
                                    client_reqwest.clone(),
                                    addr,
                                )
                                .await
                                {
                                    #[cfg(debug_assertions)]
                                    error!("Error: {}", err);
                                }
                            }
                            Err(err) => {
                                #[cfg(debug_assertions)]
                                error!("TLS Error: {}", err);
                            }
                        },
                    }
                });

            }
        }
    }
    Ok(())
}
