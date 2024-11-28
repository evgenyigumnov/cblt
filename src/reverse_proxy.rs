use crate::{matches_pattern, CbltError};
use bytes::BytesMut;
use http::{Request, StatusCode};
use log::debug;
use log::error;
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
    directive: &Directive,
) -> Result<StatusCode, CbltError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let options = match directive {
        Directive::ReverseProxy {
            pattern: _,
            destinations: _,
            options,
        } => options,
        _ => {
            return Err(CbltError::DirectiveNotMatched);
        }
    };
    for (pattern, reverse_proxy_state) in states {
        if matches_pattern(pattern, request.uri().path()) {
            loop {
                match reverse_proxy_state.get_next_backend(addr).await {
                    Ok(backend) => {
                        #[cfg(debug_assertions)]
                        debug!("Selected backend: {:?}", backend);
                        let mut dest_uri: heapless::String<{ 2 * HEAPLESS_STRING_SIZE }> =
                            heapless::String::new();
                        dest_uri
                            .push_str(backend.address.as_str())
                            .map_err(|_| CbltError::HeaplessError {})?;
                        dest_uri
                            .push_str(request.uri().path())
                            .map_err(|_| CbltError::HeaplessError {})?;

                        #[cfg(debug_assertions)]
                        debug!("Destination URI: {}", dest_uri);

                        // Parse the destination URI
                        let dest_uri_parsed = dest_uri.parse::<http::Uri>().map_err(|e| {
                            CbltError::ResponseError {
                                details: e.to_string(),
                                status_code: StatusCode::BAD_GATEWAY,
                            }
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
                        #[cfg(debug_assertions)]
                        debug!("Connecting to backend at {}", backend_addr);

                        // Establish a TCP connection to the backend with retries
                        let timeout_duration = Duration::from_secs(options.lb_timeout);
                        let mut retries = options.lb_retries;
                        let mut backend_stream_result = Err(CbltError::ResponseError {
                            details: "Failed to connect to backend".to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        });

                        while retries > 0 {
                            match timeout(
                                timeout_duration,
                                TcpStream::connect(backend_addr.as_str()),
                            )
                            .await
                            {
                                Ok(connect_result) => match connect_result {
                                    Ok(stream) => {
                                        backend_stream_result = Ok(stream);
                                        break;
                                    }
                                    Err(e) => {
                                        #[cfg(debug_assertions)]
                                        error!("Failed to connect to backend: {}", e);
                                        retries -= 1;
                                    }
                                },
                                Err(e) => {
                                    #[cfg(debug_assertions)]
                                    error!("Connection to backend timed out: {}", e);
                                    retries -= 1;
                                }
                            }
                        }

                        match backend_stream_result {
                            Ok(mut backend_stream) => {
                                // Backend is alive, update its state
                                reverse_proxy_state.set_alive_backend(&backend).await?;

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
                                let header_len =
                                    get_header_len(&mut backend_stream, &mut backend_buf).await?;

                                // Send the response headers back to the client
                                socket.write_all(&backend_buf[..header_len]).await.map_err(
                                    |e| CbltError::ResponseError {
                                        details: e.to_string(),
                                        status_code: StatusCode::BAD_GATEWAY,
                                    },
                                )?;

                                // If there's any body data already read, send it
                                if backend_buf.len() > header_len {
                                    socket.write_all(&backend_buf[header_len..]).await.map_err(
                                        |e| CbltError::ResponseError {
                                            details: e.to_string(),
                                            status_code: StatusCode::BAD_GATEWAY,
                                        },
                                    )?;
                                }

                                let (mut backend_read_half, mut backend_write_half) =
                                    backend_stream.split();
                                let (mut client_read_half, mut client_write_half) =
                                    tokio::io::split(socket);

                                let client_to_backend = async {
                                    let result = tokio::io::copy(
                                        &mut client_read_half,
                                        &mut backend_write_half,
                                    )
                                    .await;
                                    backend_write_half.shutdown().await.ok();
                                    result
                                };

                                let backend_to_client = async {
                                    let result = tokio::io::copy(
                                        &mut backend_read_half,
                                        &mut client_write_half,
                                    )
                                    .await;
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
                                            details:
                                                "Failed to copy data between client and backend"
                                                    .to_string(),
                                            status_code: StatusCode::BAD_GATEWAY,
                                        });
                                    }
                                }
                            }
                            Err(_) => {
                                // Mark the backend as dead and continue to the next backend
                                reverse_proxy_state.set_dead_backend(&backend).await?;
                                continue; // Try the next backend
                            }
                        }
                    }
                    Err(_) => {
                        return Err(CbltError::ResponseError {
                            details: "No healthy backends".to_string(),
                            status_code: StatusCode::BAD_GATEWAY,
                        });
                    }
                }
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

use crate::config::{Directive, LoadBalancePolicy, ReverseProxyOptions};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub enum AliveState {
    Alive(u64), // timestamp
    Dead {
        since: u64,        // timestamp when marked dead
        retries_left: u64, // retries remaining
    },
}
#[derive(Debug, Clone)]
pub struct Backend {
    pub url: String,
    pub alive_state: Arc<RwLock<AliveState>>,
}

pub struct ReverseProxyState {
    pub backends: Vec<Backend>,
    pub lb_policy: LoadBalancePolicy,
    pub current_backend: Arc<RwLock<usize>>, // For Round Robin
    pub options: ReverseProxyOptions,
}
#[derive(Debug, Clone)]
pub struct LiveBackend {
    address: heapless::String<HEAPLESS_STRING_SIZE>,
    backend_index: usize,
}

impl ReverseProxyState {
    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub fn new(
        backends: Vec<String>,
        lb_policy: LoadBalancePolicy,
        options: ReverseProxyOptions,
    ) -> Result<Self, CbltError> {
        let now_timestamp_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        Ok(Self {
            backends: backends
                .into_iter()
                .map(|url| Backend {
                    url,
                    alive_state: Arc::new(RwLock::new(AliveState::Alive(now_timestamp_seconds))),
                })
                .collect(),
            lb_policy,
            current_backend: Arc::new(RwLock::new(0)),
            options: options.clone(),
        })
    }
    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn set_dead_backend(&self, live_backend: &LiveBackend) -> Result<(), CbltError> {
        let now_timestamp_seconds = current_timestamp_seconds();
        let backend = &self.backends[live_backend.backend_index];
        *backend.alive_state.write().await = AliveState::Dead {
            since: now_timestamp_seconds,
            retries_left: self.options.lb_retries,
        };
        Ok(())
    }

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn set_alive_backend(&self, live_backend: &LiveBackend) -> Result<(), CbltError> {
        let now_timestamp_seconds = current_timestamp_seconds();
        let backend = &self.backends[live_backend.backend_index];
        *backend.alive_state.write().await = AliveState::Alive(now_timestamp_seconds);
        Ok(())
    }

    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    #[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
    pub async fn get_next_backend(&self, addr: SocketAddr) -> Result<LiveBackend, CbltError> {
        // Implement load balancing logic here
        match &self.lb_policy {
            LoadBalancePolicy::RoundRobin => {
                let mut idx = self.current_backend.write().await;
                let total_backends = self.backends.len();
                for _ in 0..total_backends {
                    let backend = &self.backends[*idx];
                    let mut alive_state = backend.alive_state.write().await;
                    match &mut *alive_state {
                        AliveState::Alive(_timestamp) => {
                            let live_backend = LiveBackend {
                                address: heapless::String::from_str(backend.url.as_str())
                                    .map_err(|_| CbltError::HeaplessError {})?,
                                backend_index: *idx,
                            };
                            *idx = (*idx + 1) % total_backends;
                            return Ok(live_backend);
                        }
                        AliveState::Dead {
                            since,
                            retries_left,
                        } => {
                            let now_timestamp_seconds = current_timestamp_seconds();

                            if now_timestamp_seconds > (*since + self.options.lb_interval) {
                                if *retries_left > 0 {
                                    // Attempt to bring backend back to life
                                    *retries_left -= 1;
                                    *alive_state = AliveState::Alive(now_timestamp_seconds);
                                    let live_backend = LiveBackend {
                                        address: heapless::String::from_str(backend.url.as_str())
                                            .map_err(|_| CbltError::HeaplessError {})?,
                                        backend_index: *idx,
                                    };
                                    *idx = (*idx + 1) % total_backends;
                                    return Ok(live_backend);
                                } else {
                                    // Keep backend dead
                                    *since = now_timestamp_seconds; // Reset dead since timestamp
                                }
                            }
                            *idx = (*idx + 1) % total_backends;
                        }
                    }
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
                let mut backend_idx =
                    generate_number_from_octet(addr_octets, self.backends.len() as u32);
                let total_backends = self.backends.len();
                for _ in 0..total_backends {
                    let backend = &self.backends[backend_idx as usize];
                    let mut alive_state = backend.alive_state.write().await;
                    match &mut *alive_state {
                        AliveState::Alive(_timestamp) => {
                            return Ok(LiveBackend {
                                address: heapless::String::from_str(backend.url.as_str())
                                    .map_err(|_| CbltError::HeaplessError {})?,
                                backend_index: backend_idx as usize,
                            });
                        }
                        AliveState::Dead {
                            since,
                            retries_left,
                        } => {
                            let now_timestamp_seconds = current_timestamp_seconds();
                            if now_timestamp_seconds > (*since + self.options.lb_interval) {
                                if *retries_left > 0 {
                                    // Attempt to bring backend back to life
                                    *retries_left -= 1;
                                    *alive_state = AliveState::Alive(now_timestamp_seconds);
                                    return Ok(LiveBackend {
                                        address: heapless::String::from_str(backend.url.as_str())
                                            .map_err(|_| CbltError::HeaplessError {})?,
                                        backend_index: backend_idx as usize,
                                    });
                                } else {
                                    // Keep backend dead
                                    *since = now_timestamp_seconds; // Reset dead since timestamp
                                }
                            }
                            backend_idx = (backend_idx + 1) % total_backends as u32;
                        }
                    }
                }
                Err(CbltError::ResponseError {
                    details: "No healthy backends".to_string(),
                    status_code: StatusCode::BAD_GATEWAY,
                })
            }
        }
    }
}

fn current_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
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
