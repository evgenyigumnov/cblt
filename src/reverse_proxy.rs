use crate::{matches_pattern, CbltError};
use bytes::BytesMut;
use http::{Request, StatusCode};
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

                // Parse the destination URI
                let dest_uri_parsed =
                    dest_uri
                        .parse::<http::Uri>()
                        .map_err(|e| CbltError::ResponseError {
                            details: e.to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        })?;
                let host = dest_uri_parsed.host().ok_or(CbltError::ResponseError {
                    details: "Invalid destination URI".to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                })?;
                let port = dest_uri_parsed.port_u16().unwrap_or_else(|| {
                    if dest_uri_parsed.scheme_str() == Some("https") {
                        443
                    } else {
                        80
                    }
                });
                let mut backend_addr: heapless::String<{ HEAPLESS_STRING_SIZE * 2 }> =
                    heapless::String::new();
                backend_addr
                    .push_str(host)
                    .map_err(|_| CbltError::HeaplessError {})?;
                backend_addr
                    .push_str(":")
                    .map_err(|_| CbltError::HeaplessError {})?;
                backend_addr
                    .push_str(port.to_string().as_str())
                    .map_err(|_| CbltError::HeaplessError {})?;
                debug!("Connecting to backend at {}", backend_addr);

                // Establish a TCP connection to the backend
                let mut backend_stream =
                    TcpStream::connect(backend_addr.as_str())
                        .await
                        .map_err(|e| CbltError::ResponseError {
                            details: e.to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        })?;

                // Send the initial request to the backend
                let request_bytes = request_to_bytes(request)?;
                backend_stream
                    .write_all(&request_bytes)
                    .await
                    .map_err(|e| CbltError::ResponseError {
                        details: e.to_string(),
                        status_code: StatusCode::BAD_GATEWAY,
                    })?;

                // Read the response from the backend
                let mut backend_buf = BytesMut::with_capacity(8192);
                let header_len = get_header_len(&mut backend_stream, &mut backend_buf).await?;

                // Send the response headers back to the client
                socket
                    .write_all(&backend_buf[..header_len])
                    .await
                    .map_err(|e| CbltError::ResponseError {
                        details: e.to_string(),
                        status_code: StatusCode::BAD_GATEWAY,
                    })?;

                // If there's any body data already read, send it
                if backend_buf.len() > header_len {
                    socket
                        .write_all(&backend_buf[header_len..])
                        .await
                        .map_err(|e| CbltError::ResponseError {
                            details: e.to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        })?;
                }

                let (mut backend_read_half, mut backend_write_half) = backend_stream.split();
                let (mut client_read_half, mut client_write_half) = tokio::io::split(socket);

                let client_to_backend = async {
                    let result =
                        tokio::io::copy(&mut client_read_half, &mut backend_write_half).await;
                    backend_write_half.shutdown().await.ok();
                    result
                };

                let backend_to_client = async {
                    let result =
                        tokio::io::copy(&mut backend_read_half, &mut client_write_half).await;
                    client_write_half.shutdown().await.ok();
                    result
                };

                let (client_to_backend_res, backend_to_client_res) =
                    tokio::join!(client_to_backend, backend_to_client);
                match (client_to_backend_res, backend_to_client_res) {
                    (Ok(_), Ok(_)) => {
                        return Ok(StatusCode::OK);
                    }
                    _ => {
                        return Err(CbltError::ResponseError {
                            details: "Failed to copy data between client and backend".to_string(),
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

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn request_to_bytes(request: &Request<BytesMut>) -> Result<Vec<u8>, CbltError> {
    let mut buf = Vec::new();
    // Write request line
    buf.extend_from_slice(request.method().as_str().as_bytes());
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(
        request
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/")
            .as_bytes(),
    );
    buf.extend_from_slice(b" HTTP/1.1\r\n");

    // Write headers
    for (key, value) in request.headers() {
        buf.extend_from_slice(key.as_str().as_bytes());
        buf.extend_from_slice(b": ");
        buf.extend_from_slice(value.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
    buf.extend_from_slice(b"\r\n");

    // Write body
    buf.extend_from_slice(request.body());

    Ok(buf)
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
async fn get_header_len<S>(socket: &mut S, buf: &mut BytesMut) -> Result<usize, CbltError>
where
    S: AsyncReadExt + Unpin,
{
    loop {
        let bytes_read = socket.read_buf(buf).await.unwrap_or(0);
        if bytes_read == 0 {
            break;
        }
        // Try to parse the response
        let mut headers = [httparse::EMPTY_HEADER; 64]; // Increased header capacity
        let mut res = httparse::Response::new(&mut headers);

        match res.parse(buf) {
            Ok(httparse::Status::Complete(header_len)) => {
                return Ok(header_len);
            }
            Ok(httparse::Status::Partial) => {
                // Need to read more data
                continue;
            }
            Err(e) => {
                return Err(CbltError::ResponseError {
                    details: e.to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                });
            }
        }
    }

    Err(CbltError::ResponseError {
        details: "Failed to read response from backend".to_string(),
        status_code: StatusCode::BAD_GATEWAY,
    })
}

use crate::config::LoadBalancePolicy;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::TcpStream;
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
    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
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

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
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

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn start_health_checks(&self, health_uri: String, interval: u64, timeout: u64) {
        let client = self.client.clone();
        let backends = self.backends.clone();
        let is_running_clone = self.is_running_check.clone();

        tokio::spawn(async move {
            let interval = tokio::time::Duration::from_secs(interval);
            let timeout = tokio::time::Duration::from_secs(timeout);
            while is_running_clone.load(Ordering::SeqCst) {
                for backend in &backends {
                    let mut url: heapless::String<{ HEAPLESS_STRING_SIZE * 2 }> =
                        heapless::String::new();
                    if let Err(err) = url.push_str(backend.url.as_str()) {
                        log::error!("Error: {:?}", err);
                        break;
                    }
                    if let Err(err) = url.push_str(health_uri.as_str()) {
                        log::error!("Error: {:?}", err);
                        break;
                    }
                    let is_healthy = backend.is_healthy.clone();
                    let client = client.clone();
                    tokio::spawn(async move {
                        let resp = client.get(&*url).timeout(timeout).send().await;
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
#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn generate_number_from_octet(octets: [u8; 4], max: u32) -> u32 {
    let mut hash: u64 = FNV_OFFSET_BASIS;
    for byte in &octets {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    (hash % max as u64) as u32
}
