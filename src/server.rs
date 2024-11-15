use crate::buffer_pool::BufferPool;
use crate::config::Directive;
use crate::directive::directive_process;
use crate::error::CbltError;
use crate::request::BUF_SIZE;
use log::{error, info};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;

#[derive(Debug, Clone)]
pub struct Server {
    pub port: u16,
    pub hosts: HashMap<String, Vec<Directive>>, // Host -> Directives
    pub cert: Option<String>,
    pub key: Option<String>,
}

pub async fn server_init(server: &Server, max_connections: usize) -> Result<(), CbltError> {
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
