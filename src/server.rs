use crate::config::Directive;
use crate::directive::directive_process;
use crate::error::CbltError;
use std::collections::HashMap;

use log::{error, info};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;
#[cfg(feature = "trace")]
use tracing::instrument;

#[derive(Debug, Clone)]
pub struct Server {
    pub port: u16,
    pub hosts: HashMap<String, Vec<Directive>>, // Host -> Directives
    pub cert: Option<String>,
    pub key: Option<String>,
}

pub struct ServerWorker {
    pub port: u16,
    pub settings: ServerSettings,
}

#[derive(Clone)]
pub struct ServerSettings {
    pub hosts: HashMap<String, Vec<Directive>>,
    pub tls_acceptor: Option<TlsAcceptor>,
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
    pub fn new(server: Server) -> Result<Self, CbltError> {
        let tls_acceptor = tls_acceptor_builder(server.cert.as_deref(), server.key.as_deref())?;
        Ok(ServerWorker {
            port: server.port,
            settings: ServerSettings {
                hosts: server.hosts,
                tls_acceptor,
            },
        })
    }

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn run(&self, max_connections: usize) -> Result<(), CbltError> {
        let port = self.port;
        let acceptor = self.settings.tls_acceptor.clone();
        let hosts = self.settings.hosts.clone();

        tokio::spawn(async move {
            if let Err(err) = init_server(port, acceptor, hosts, max_connections).await {
                error!("Error: {}", err);
            }
        });
        Ok(())
    }

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn update(
        &mut self,
        hosts: HashMap<String, Vec<Directive>>,
        cert_path: Option<String>,
        key_path: Option<String>,
    ) -> Result<(), CbltError> {
        let cert_path_opt = cert_path.as_deref();
        let key_path_opt = key_path.as_deref();
        let tls_acceptor = tls_acceptor_builder(cert_path_opt, key_path_opt)?;

        self.settings.hosts = hosts;
        self.settings.tls_acceptor = tls_acceptor;
        Ok(())
    }
}

async fn init_server(
    port: u16,
    acceptor: Option<TlsAcceptor>,
    hosts: HashMap<String, Vec<Directive>>,
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
        let acceptor = acceptor.clone();
        let hosts = hosts.clone();
        tokio::spawn(async move {
            let _permit = permit;

            match acceptor {
                None => {
                    if let Err(err) =
                        directive_process(&mut stream, &hosts, client_reqwest.clone())
                            .await
                    {
                        #[cfg(debug_assertions)]
                        error!("Error: {}", err);
                    }
                }
                Some(ref acceptor) => match acceptor.accept(stream).await {
                    Ok(mut stream) => {
                        if let Err(err) = directive_process(
                            &mut stream,
                            &hosts,
                            client_reqwest.clone(),
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
