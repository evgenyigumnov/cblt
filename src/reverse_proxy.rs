use crate::response::send_response_stream;
use crate::{matches_pattern, CbltError};
use bytes::BytesMut;
use http::{Request, Response, StatusCode};
use log::debug;
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(feature = "trace")]
use tracing::instrument;
pub const HEAPLESS_STRING_SIZE: usize = 100;

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub async fn proxy_directive<S>(
    request: &Request<BytesMut>,
    socket: &mut S,
    client_reqwest: Client,
    states: &HashMap<String, ReverseProxyState>,
    addr: SocketAddr,
) -> Result<StatusCode, CbltError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    for (pattern, reverse_proxy_stat) in states {
        if matches_pattern(pattern, request.uri().path()) {
            if let Ok(destination) = reverse_proxy_stat.get_next_backend(addr).await {
                debug!("Selected backend: {:?}", destination);
                let mut dest_uri: heapless::String<{ 2 * HEAPLESS_STRING_SIZE }> =
                    heapless::String::new();
                dest_uri
                    .push_str(destination.as_str())
                    .map_err(|_| CbltError::HeaplessError {})?;
                dest_uri
                    .push_str(request.uri().path())
                    .map_err(|_| CbltError::HeaplessError {})?;

                #[cfg(debug_assertions)]
                debug!("Destination URI: {}", dest_uri);

                let mut req_builder =
                    client_reqwest.request(request.method().clone(), dest_uri.as_str());
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
                            return Err(CbltError::ResponseError {
                                details: "Bad gateway".to_string(),
                                status_code: status,
                            });
                        } else {
                            return Ok(status);
                        }
                    }
                    Err(err) => {
                        #[cfg(debug_assertions)]
                        debug!("Error: {:?}", err);
                        return Err(CbltError::ResponseError {
                            details: err.to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        });
                    }
                }
            } else {
                return Err(CbltError::ResponseError {
                    details: "No healthy backends".to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                });
            }
        }
    }

    Err(CbltError::DirectiveNotMatched)
}

use crate::config::LoadBalancePolicy;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct Backend {
    pub url: String,
    pub is_healthy: Arc<RwLock<bool>>,
}

pub struct ReverseProxyState {
    pub backends: Vec<Backend>,
    pub lb_policy: LoadBalancePolicy,
    pub current_backend: Arc<RwLock<usize>>, // For Round Robin
    pub client: Client,
    pub is_running_check: Arc<AtomicBool>,
}

impl ReverseProxyState {
    pub fn new(backends: Vec<String>, lb_policy: LoadBalancePolicy, client: Client) -> Self {
        Self {
            backends: backends
                .into_iter()
                .map(|url| Backend {
                    url,
                    is_healthy: Arc::new(RwLock::new(true)),
                })
                .collect(),
            lb_policy,
            current_backend: Arc::new(RwLock::new(0)),
            client,
            is_running_check: Arc::new(AtomicBool::new(true)),
        }
    }

    pub async fn get_next_backend(
        &self,
        addr: SocketAddr,
    ) -> Result<heapless::String<HEAPLESS_STRING_SIZE>, CbltError> {
        // Implement load balancing logic here
        match &self.lb_policy {
            LoadBalancePolicy::RoundRobin => {
                let mut idx = self.current_backend.write().await;
                let total_backends = self.backends.len();
                for _ in 0..total_backends {
                    let backend = &self.backends[*idx];
                    if *backend.is_healthy.read().await {
                        *idx = (*idx + 1) % total_backends;
                        return heapless::String::from_str(backend.url.as_str())
                            .map_err(|_| CbltError::HeaplessError {});
                    }
                    *idx = (*idx + 1) % total_backends;
                }
                Err(CbltError::ResponseError {
                    details: "No healthy backends".to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                })
            }
            LoadBalancePolicy::IPHash => {
                let addr_octets = match addr.ip() {
                    IpAddr::V4(addr) => addr.octets(),
                    IpAddr::V6(..) => {
                        return Err(CbltError::ResponseError {
                            details: "IPv6 not supported".to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        });
                    }
                };
                let backend_idx =
                    generate_number_from_octet(addr_octets, self.backends.len() as u32);
                let backend = &self.backends[backend_idx as usize];
                if *backend.is_healthy.read().await {
                    return heapless::String::from_str(backend.url.as_str())
                        .map_err(|_| CbltError::HeaplessError {});
                } else if backend_idx == self.backends.len() as u32 - 1 {
                    let backend = &self.backends[0_usize];
                    if *backend.is_healthy.read().await {
                        return heapless::String::from_str(backend.url.as_str())
                            .map_err(|_| CbltError::HeaplessError {});
                    } else {
                        Err(CbltError::ResponseError {
                            details: "No healthy backends".to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        })
                    }
                } else {
                    let backend = &self.backends[(backend_idx + 1) as usize];
                    if *backend.is_healthy.read().await {
                        return heapless::String::from_str(backend.url.as_str())
                            .map_err(|_| CbltError::HeaplessError {});
                    } else {
                        Err(CbltError::ResponseError {
                            details: "No healthy backends".to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        })
                    }
                }
            }
        }
    }

    pub async fn start_health_checks(&self, health_uri: String, interval: u64, timeout: u64) {
        let client = self.client.clone();
        let backends = self.backends.clone();
        let is_running_clone = self.is_running_check.clone();

        tokio::spawn(async move {
            let interval = tokio::time::Duration::from_secs(interval);
            let timeout = tokio::time::Duration::from_secs(timeout);
            while is_running_clone.load(Ordering::SeqCst) {
                for backend in &backends {
                    let url = format!("{}{}", backend.url, health_uri);
                    let is_healthy = backend.is_healthy.clone();
                    let client = client.clone();
                    tokio::spawn(async move {
                        let resp = client.get(&url).timeout(timeout).send().await;
                        let mut health = is_healthy.write().await;
                        *health = resp.is_ok()
                            && match resp {
                                Ok(rest) => rest.status().is_success(),
                                Err(err) => {
                                    #[cfg(debug_assertions)]
                                    debug!("Error: {:?}", err);
                                    false
                                }
                            };
                        #[cfg(debug_assertions)]
                        debug!("Health check for {}: {}", url, *health);
                    });
                }
                tokio::time::sleep(interval).await;
            }
        });
    }
}

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
fn generate_number_from_octet(octets: [u8; 4], max: u32) -> u32 {
    let mut hash: u64 = FNV_OFFSET_BASIS;
    for byte in &octets {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    (hash % max as u64) as u32
}
