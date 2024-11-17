use crate::buffer_pool::{BufferManager, Pool};
use crate::config::Directive;
use crate::directive::directive_process;
use crate::error::CbltError;
use crate::request::BUF_SIZE;
use heapless::FnvIndexMap;
use log::{error, info};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::mem;
use std::sync::Arc;
use deadpool::managed;
use deadpool::managed::{PoolConfig};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio_rustls::TlsAcceptor;

pub const STRING_CAPACITY: usize = 200;
pub const DIRECTIVE_CAPACITY: usize = 10;
pub const HOST_CAPACITY: usize = 8;

#[derive(Debug, Clone)]
pub struct Server {
    pub port: u16,
    pub hosts: FnvIndexMap<
        heapless::String<STRING_CAPACITY>,
        heapless::Vec<Directive, DIRECTIVE_CAPACITY>,
        HOST_CAPACITY,
    >, // Host -> Directives
    pub cert: Option<heapless::String<STRING_CAPACITY>>,
    pub key: Option<heapless::String<STRING_CAPACITY>>,
}

pub struct ServerWorker {
    pub port: u16,
    pub settings: Arc<RwLock<ServerSettings>>,
}

pub struct ServerSettings {
    pub hosts: FnvIndexMap<
        heapless::String<STRING_CAPACITY>,
        heapless::Vec<Directive, DIRECTIVE_CAPACITY>,
        HOST_CAPACITY,
    >,
    pub tls_acceptor: Option<Arc<TlsAcceptor>>,
}

fn tls_acceptor_bulder(
    cert_path: Option<&str>,
    key_path: Option<&str>,
) -> Result<Option<Arc<TlsAcceptor>>, CbltError> {
    if let (Some(cert_path), Some(key_path)) = (cert_path, key_path) {
        let certs = CertificateDer::pem_file_iter(cert_path)?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(key_path)?;

        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        Ok(Some(Arc::new(TlsAcceptor::from(Arc::new(server_config)))))
    } else {
        Ok(None)
    }
}
impl ServerWorker {
    pub fn new(server: Server) -> Result<Self, CbltError> {
        let tls_acceptor = tls_acceptor_bulder(server.cert.as_deref(), server.key.as_deref())?;
        Ok(ServerWorker {
            port: server.port,
            settings: Arc::new(RwLock::new(ServerSettings {
                hosts: server.hosts,
                tls_acceptor,
            })),
        })
    }

    pub async fn run(&self, max_connections: usize) -> Result<(), CbltError> {
        let semaphore = Arc::new(Semaphore::new(max_connections));
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;
        let manager = BufferManager{
            buffer_size: BUF_SIZE,
        };

        let pool = Pool::builder(manager)
            .config(PoolConfig {
                max_size: max_connections, // Максимальное количество буферов в пуле
                ..Default::default()
            })
            .build()?;
        let buffer_pool = Arc::new(pool);
        info!("Listening on port: {}", self.port);
        let client_reqwest = reqwest::Client::new();

        loop {
            let client_reqwest = client_reqwest.clone();
            let buffer_pool_arc = buffer_pool.clone();
            let server_clone = self.clone();
            let (mut stream, _) = listener.accept().await?;
            let permit = semaphore.clone().acquire_owned().await?;

            tokio::spawn(async move {
                let _permit = permit;
                let buffer = buffer_pool_arc.get().await.unwrap();
                let acceptor = server_clone.settings.read().await.tls_acceptor.clone();
                let hosts = server_clone.settings.read().await.hosts.clone();

                match acceptor {
                    None => {
                        if let Err(err) = directive_process(
                            &mut stream,
                            &hosts,
                            buffer,
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
                                &hosts,
                                buffer,
                                client_reqwest.clone(),
                            )
                            .await
                            {
                                error!("Error: {}", err);
                            }
                        }
                        Err(err) => {
                            error!("TLS Error: {}", err);
                        }
                    },
                }
            });
        }
    }

    /// Updates the server settings except for the port.
    pub async fn update(
        &self,
        hosts: FnvIndexMap<
            heapless::String<STRING_CAPACITY>,
            heapless::Vec<Directive, DIRECTIVE_CAPACITY>,
            HOST_CAPACITY,
        >,
        cert_path: Option<heapless::String<STRING_CAPACITY>>,
        key_path: Option<heapless::String<STRING_CAPACITY>>,
    ) -> Result<(), CbltError> {
        let cert_path_opt = cert_path.as_deref();
        let key_path_opt = key_path.as_deref();
        let tls_acceptor = tls_acceptor_bulder(cert_path_opt, key_path_opt)?;

        let mut settings = self.settings.write().await;
        settings.hosts = hosts;
        settings.tls_acceptor = tls_acceptor;
        Ok(())
    }
}


impl Clone for ServerWorker {
    fn clone(&self) -> Self {
        ServerWorker {
            port: self.port,
            settings: self.settings.clone(),
        }
    }
}
