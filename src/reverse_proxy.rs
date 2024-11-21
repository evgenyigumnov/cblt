use crate::response::send_response_stream;
use crate::{matches_pattern, CbltError};
use bytes::BytesMut;
use http::{Request, Response, StatusCode};
use log::debug;
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(feature = "trace")]
use tracing::instrument;

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub async fn proxy_directive<S>(
    request: &Request<BytesMut>,
    socket: &mut S,
    pattern: &str,
    destination: &str,
    client_reqwest: Client,
) -> Result<StatusCode, CbltError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    if matches_pattern(pattern, request.uri().path()) {
        //let dest_uri = [destination, request.uri().path()].concat();
        let mut dest_uri: heapless::String<200> = heapless::String::new();
        dest_uri
            .push_str(destination)
            .map_err(|_| CbltError::HeaplessError {})?;
        dest_uri
            .push_str(request.uri().path())
            .map_err(|_| CbltError::HeaplessError {})?;

        #[cfg(debug_assertions)]
        debug!("Destination URI: {}", dest_uri);

        let mut req_builder = client_reqwest.request(request.method().clone(), dest_uri.as_str());
        for (key, value) in request.headers().iter() {
            req_builder = req_builder.header(key, value);
        }
        let body = request.body();
        if !body.is_empty() {
            req_builder = req_builder.body(body.to_vec());
        }

        match req_builder.send().await {
            Ok(resp) => {
                let status = resp.status();
                let headers = resp.headers();
                let mut response_builder = Response::builder().status(status);
                for (key, value) in headers.iter() {
                    response_builder = response_builder.header(key, value);
                }
                let mut stream = resp.bytes_stream();
                let response = response_builder.body("")?;
                send_response_stream(socket, response, request, &mut stream).await?;
                if status != StatusCode::OK {
                    Err(CbltError::ResponseError {
                        details: "Bad gateway".to_string(),
                        status_code: status,
                    })
                } else {
                    Ok(status)
                }
            }
            Err(err) => {
                #[cfg(debug_assertions)]
                debug!("Error: {:?}", err);
                Err(CbltError::ResponseError {
                    details: err.to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                })
            }
        }
    } else {
        Err(CbltError::DirectiveNotMatched)
    }
}


use tokio::sync::RwLock;
use std::sync::Arc;
use std::collections::HashMap;
use crate::config::LoadBalancePolicy;

#[derive(Clone)]
pub struct Backend {
    pub url: String,
    pub is_healthy: Arc<RwLock<bool>>,
}

pub struct ReverseProxyState {
    pub backends: Vec<Backend>,
    pub lb_policy: LoadBalancePolicy,
    pub current_backend: Arc<RwLock<usize>>, // For Round Robin
    pub client: Client,
}

impl ReverseProxyState {
    pub fn new(backends: Vec<String>, lb_policy: LoadBalancePolicy, client: Client) -> Self {
        Self {
            backends: backends.into_iter().map(|url| Backend {
                url,
                is_healthy: Arc::new(RwLock::new(true)),
            }).collect(),
            lb_policy,
            current_backend: Arc::new(RwLock::new(0)),
            client,
        }
    }

    pub async fn get_next_backend(&self, request: &Request<BytesMut>) -> Option<Backend> {
        // Implement load balancing logic here
        match &self.lb_policy {
            LoadBalancePolicy::RoundRobin => {
                let mut idx = self.current_backend.write().await;
                let total_backends = self.backends.len();
                for _ in 0..total_backends {
                    let backend = &self.backends[*idx];
                    if *backend.is_healthy.read().await {
                        let selected_backend = backend.clone();
                        *idx = (*idx + 1) % total_backends;
                        return Some(selected_backend);
                    }
                    *idx = (*idx + 1) % total_backends;
                }
                None
            },
            LoadBalancePolicy::Cookie { cookie_name, .. } => {
                // Check for the cookie in the request
                if let Some(cookie_header) = request.headers().get("Cookie") {
                    if let Ok(cookie_str) = cookie_header.to_str() {
                        for cookie in cookie_str.split(';') {
                            let cookie = cookie.trim();
                            if cookie.starts_with(cookie_name) {
                                let parts: Vec<&str> = cookie.split('=').collect();
                                if parts.len() == 2 {
                                    if let Ok(backend_idx) = parts[1].parse::<usize>() {
                                        if backend_idx < self.backends.len() {
                                            let backend = &self.backends[backend_idx];
                                            if *backend.is_healthy.read().await {
                                                return Some(backend.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // If no valid cookie, fallback to Round Robin or another method
                self.get_next_backend_round_robin().await
            },
        }
    }

    async fn get_next_backend_round_robin(&self) -> Option<Backend> {
        // Similar to the Round Robin implementation above
        // ...

        None
    }

    pub async fn start_health_checks(&self, health_uri: String, interval: u64, timeout: u64) {
        let client = self.client.clone();
        let backends = self.backends.clone();

        tokio::spawn(async move {
            let interval = tokio::time::Duration::from_secs(interval);
            let timeout = tokio::time::Duration::from_secs(timeout);
            loop {
                for backend in &backends {
                    let url = format!("{}{}", backend.url, health_uri);
                    let is_healthy = backend.is_healthy.clone();
                    let client = client.clone();
                    tokio::spawn(async move {
                        let resp = client.get(&url).timeout(timeout).send().await;
                        let mut health = is_healthy.write().await;
                        *health = resp.is_ok() && resp.unwrap().status().is_success();
                    });
                }
                tokio::time::sleep(interval).await;
            }
        });
    }
}