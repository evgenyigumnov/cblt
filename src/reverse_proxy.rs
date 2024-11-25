use crate::response::send_response_stream;
use crate::{matches_pattern, CbltError};
use bytes::BytesMut;
use http::{Request, Response, StatusCode};
use log::debug;
use reqwest::Client;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
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

                if is_websocket_upgrade(request) {
                    // Handle WebSocket upgrade request
                    // Establish a TCP connection to the backend
                    let dest_uri_parsed = dest_uri.parse::<http::Uri>()
                        .map_err(|e| CbltError::ResponseError {
                            details: e.to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        })?;
                    let host = dest_uri_parsed.host().ok_or(CbltError::ResponseError {
                        details: "Invalid destination URI".to_string(),
                        status_code: StatusCode::BAD_GATEWAY,
                    })?;
                    let port = dest_uri_parsed.port_u16().unwrap_or(80); // Adjust if needed for https
                    let backend_addr = format!("{}:{}", host, port);
                    debug!("Connecting to backend at {}", backend_addr);
                    let mut backend_stream = TcpStream::connect(backend_addr).await.map_err(|e| CbltError::ResponseError {
                        details: e.to_string(),
                        status_code: StatusCode::BAD_GATEWAY,
                    })?;

                    // Send the initial request to the backend
                    let request_bytes = request_to_bytes(request)?;
                    backend_stream.write_all(&request_bytes).await.map_err(|e| CbltError::ResponseError {
                        details: e.to_string(),
                        status_code: StatusCode::BAD_GATEWAY,
                    })?;

                    // Read the response from the backend
                    let mut backend_buf = BytesMut::with_capacity(8192);
                        let response = socket_to_response(&mut backend_stream, &mut backend_buf).await?;

                    // Send the response back to the client
                    socket.write_all(&backend_buf).await.map_err(|e| CbltError::ResponseError {
                        details: e.to_string(),
                        status_code: StatusCode::BAD_GATEWAY,
                    })?;

                    // If the response is 101 Switching Protocols, start relaying data
                    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
                        // Relay data between client and backend
                        let (mut backend_read_half, mut backend_write_half) = backend_stream.split();
                        let (mut client_read_half, mut client_write_half) =  tokio::io::split(socket);
                        let client_to_backend = tokio::io::copy(&mut client_read_half, &mut backend_write_half);
                        let backend_to_client = tokio::io::copy(&mut backend_read_half, &mut client_write_half);
                        tokio::try_join!(client_to_backend, backend_to_client).map_err(|e| CbltError::ResponseError {
                            details: e.to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        })?;
                        return Ok(StatusCode::SWITCHING_PROTOCOLS);
                    } else {
                        // Not a successful WebSocket handshake
                        return Ok(response.status());
                    }
                } else {
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

fn is_websocket_upgrade(request: &Request<BytesMut>) -> bool {
    if let Some(connection_header) = request.headers().get("Connection") {
        if connection_header.to_str().map(|s| s.to_ascii_lowercase().contains("upgrade")).unwrap_or(false) {
            if let Some(upgrade_header) = request.headers().get("Upgrade") {
                if upgrade_header.to_str().map(|s| s.to_ascii_lowercase() == "websocket").unwrap_or(false) {
                    return true;
                }
            }
        }
    }
    false
}


fn request_to_bytes(request: &Request<BytesMut>) -> Result<Vec<u8>, CbltError> {
    let mut buf = Vec::new();
    // Write request line
    buf.extend_from_slice(request.method().as_str().as_bytes());
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(request.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/").as_bytes());
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
    buf.extend_from_slice(&request.body());

    Ok(buf)
}

async fn socket_to_response<S>(socket: &mut S, buf: &mut BytesMut) -> Result<Response<()>, CbltError>
    where
        S: AsyncReadExt + Unpin,
{
    loop {
        let bytes_read = socket.read_buf(buf).await.unwrap_or(0);
        if bytes_read == 0 {
            break;
        }
        // Try to parse the response
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut res = httparse::Response::new(&mut headers);

        match res.parse(buf) {
            Ok(httparse::Status::Complete(header_len)) => {
                let status_code = res.code.ok_or(CbltError::ResponseError {
                    details: "Failed to parse status code".to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                })?;
                let status = StatusCode::from_u16(status_code).map_err(|e| CbltError::ResponseError {
                    details: e.to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                })?;
                let version = res.version.ok_or(CbltError::ResponseError {
                    details: "Failed to parse HTTP version".to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                })?;
                let mut response_builder = Response::builder().status(status);
                for header in res.headers {
                    response_builder = response_builder.header(header.name, header.value);
                }
                let response = response_builder.body(()).map_err(|e| CbltError::ResponseError {
                    details: e.to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                })?;
                return Ok(response);
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
