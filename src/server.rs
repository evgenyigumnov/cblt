use std::collections::hash_map::Entry;
use crate::config::{Directive, LoadBalancePolicy};
use crate::directive::directive_process;
use crate::error::CbltError;
use std::collections::HashMap;

use log::{error, info};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use humantime::Duration;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, Semaphore};
use tokio_rustls::TlsAcceptor;
#[cfg(feature = "trace")]
use tracing::instrument;
use crate::reverse_proxy::ReverseProxyState;

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
}

pub struct SettingsLock {
    settings: RwLock<Arc<ServerSettings>>,
}

impl SettingsLock {
    async fn update(&self, s: Arc<ServerSettings>) {
        let mut settings = self.settings.write().await;
        *settings = s;
    }
    async fn get(&self) -> Arc<ServerSettings> {
        let settings = self.settings.read().await;
        settings.clone()
    }
}

#[derive(Clone)]
pub struct ServerSettings {
    pub hosts: Arc<HashMap<String, Vec<Directive>>>,
    pub tls_acceptor: Arc<Option<TlsAcceptor>>,
    pub reverse_proxy_states: Arc<HashMap<String, HashMap<String, ReverseProxyState>>>
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

        let reverse_proxy_states = init_proxy_states(&server.hosts).await;


        Ok(ServerWorker {
            port: server.port,
            lock: Arc::new(SettingsLock {
                settings: RwLock::new(
                    ServerSettings {
                        hosts: server.hosts.into(),
                        tls_acceptor: tls_acceptor.into(),
                        reverse_proxy_states: Arc::new(reverse_proxy_states?),
                    }
                    .into(),
                ),
            }),
        })
    }

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn run(&self, max_connections: usize) -> Result<(), CbltError> {
        let port = self.port;
        let settings = self.lock.clone();

        tokio::spawn(async move {
            if let Err(err) = init_server(port, settings, max_connections).await {
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
        let reverse_proxy_states = init_proxy_states(&hosts).await;
        self.lock
            .update(
                ServerSettings {
                    hosts: hosts.into(),
                    tls_acceptor: tls_acceptor.into(),
                    reverse_proxy_states: Arc::new(reverse_proxy_states?),
                }
                .into(),
            )
            .await;
        Ok(())
    }
}

async fn init_proxy_states(hosts: &HashMap<String, Vec<Directive>>) -> Result<HashMap<String, HashMap<String, ReverseProxyState>>, CbltError> {
    let mut reverse_proxy_states:HashMap<String, HashMap<String, ReverseProxyState>> = HashMap::new(); // host -> (pattern -> ReverseProxyState)
    let client_reqwest = reqwest::Client::new();
    for host in hosts.keys() {
        let directives = hosts.get(host).ok_or(CbltError::DirectivesNotFound)?;
        for directive in directives {
            match directive {
                Directive::ReverseProxy { pattern, destinations, options } => {
                    let reverse_proxy_state = ReverseProxyState::new(
                        destinations.clone(),
                        options.lb_policy.clone().unwrap_or(LoadBalancePolicy::RoundRobin),
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

                        match reverse_proxy_states.entry(host.clone()) {
                            Entry::Occupied(mut entry) => {
                                entry.get_mut().insert(pattern.clone(), reverse_proxy_state);
                            }

                            Entry::Vacant(entry) => {
                                let mut map = HashMap::new();
                                map.insert(pattern.clone(), reverse_proxy_state);
                                entry.insert(map);
                            }
                        }

                    }
                }
                _ => continue,
            }

        }
    }
    Ok(reverse_proxy_states)
}

async fn init_server(
    port: u16,
    settings_lock: Arc<SettingsLock>,
    max_connections: usize,
) -> Result<(), CbltError> {
    let semaphore = Arc::new(Semaphore::new(max_connections));
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on port: {}", port);
    let client_reqwest = reqwest::Client::new();

    loop {
        let client_reqwest = client_reqwest.clone();
        let (mut stream, _) = listener.accept().await?;
        let permit = semaphore.clone().acquire_owned().await?;
        let settings = settings_lock.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let settings = settings.get().await;
            let hosts = settings.hosts.clone();
            let acceptor = settings.tls_acceptor.clone();
            match acceptor.as_ref() {
                None => {
                    if let Err(err) =
                        directive_process(&mut stream, &hosts, client_reqwest.clone()).await
                    {
                        #[cfg(debug_assertions)]
                        error!("Error: {}", err);
                    }
                }
                Some(ref acceptor) => match acceptor.accept(stream).await {
                    Ok(mut stream) => {
                        if let Err(err) =
                            directive_process(&mut stream, &hosts, client_reqwest.clone()).await
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
