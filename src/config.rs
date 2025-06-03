use crate::error::CbltError;
use crate::server::Server;
use crate::{build_servers, Args};
use bollard::container::ListContainersOptions;
use bollard::service::ListServicesOptions;
use kdl::{KdlDocument, KdlNode};
use log::debug;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::fs;
#[cfg(feature = "trace")]
use tracing::instrument;

#[derive(Debug, Clone)]
pub enum Directive {
    Root {
        pattern: String,
        path: String,
        fallback: Option<String>, // fallback file path
    },
    FileServer,
    ReverseProxy {
        pattern: String,
        destinations: Vec<String>,
        options: ReverseProxyOptions,
    },
    Redir {
        destination: String,
    },
    RedirIfNotCookie {
        cookiename: String,
        destination: String,
    },
    TlS {
        cert: String,
        key: String,
    },
}

#[derive(Debug, Clone)]
pub enum LoadBalancePolicy {
    RoundRobin,
    IPHash,
}

#[derive(Debug, Clone, Default)]
pub struct ReverseProxyOptions {
    pub lb_retries: u64,
    pub lb_interval: u64,
    pub lb_timeout: u64,
    pub lb_policy: Option<LoadBalancePolicy>,
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub fn build_config(doc: &KdlDocument) -> Result<HashMap<String, Vec<Directive>>, CbltError> {
    let mut hosts = HashMap::new();

    for node in doc.nodes() {
        let hostname = node.name().value().to_string();
        let mut directives = Vec::new();

        if let Some(children) = node.children() {
            for child_node in children.nodes() {
                let child_name = child_node.name().value();

                match child_name {
                    "root" => {
                        let args = get_string_args(child_node);
                        if args.len() >= 2 {
                            let pattern = args[0].to_string();
                            let path = args[1].to_string();
                            let fallback = if args.len() >= 3 {
                                Some(args[2].to_string())
                            } else {
                                None
                            };
                            directives.push(Directive::Root { pattern, path, fallback });
                        } else {
                            return Err(CbltError::KdlParseError {
                                details: format!("Invalid 'root' directive for host {}", hostname),
                            });
                        }
                    }
                    "file_server" => {
                        directives.push(Directive::FileServer);
                    }
                    "reverse_proxy" => {
                        let args = get_string_args(child_node);
                        if args.len() >= 2 {
                            let pattern = args[0].to_string();
                            let destinations = args[1..].iter().map(|s| s.to_string()).collect();

                            let options = parse_reverse_proxy_options(child_node)?;
                            directives.push(Directive::ReverseProxy {
                                pattern,
                                destinations,
                                options,
                            });
                        } else {
                            return Err(CbltError::KdlParseError {
                                details: format!(
                                    "Invalid 'reverse_proxy' directive for host {}",
                                    hostname
                                ),
                            });
                        }
                    }
                    "redir" => {
                        let args = get_string_args(child_node);
                        if !args.is_empty() {
                            let destination = args[0].to_string();
                            directives.push(Directive::Redir { destination });
                        } else {
                            return Err(CbltError::KdlParseError {
                                details: format!("Invalid 'redir' directive for host {}", hostname),
                            });
                        }
                    }
                    "redirifnotcookie" => {
                        let args = get_string_args(child_node);
                        if !args.is_empty() {
                            let cookiename = args[0].to_string();
                            let destination = args[1].to_string();
                            directives.push(Directive::RedirIfNotCookie {
                                cookiename,
                                destination,
                            });
                        } else {
                            return Err(CbltError::KdlParseError {
                                details: format!("Invalid 'redir' directive for host {}", hostname),
                            });
                        }
                    }

                    "tls" => {
                        let args = get_string_args(child_node);
                        if args.len() >= 2 {
                            let cert = args[0].to_string();
                            let key = args[1].to_string();
                            directives.push(Directive::TlS { cert, key });
                        } else {
                            return Err(CbltError::KdlParseError {
                                details: format!("Invalid 'tls' directive for host {}", hostname),
                            });
                        }
                    }
                    _ => {
                        return Err(CbltError::KdlParseError {
                            details: format!(
                                "Unknown directive '{}' for host {}",
                                child_name, hostname
                            ),
                        });
                    }
                }
            }
        }

        if directives.is_empty() {
            return Err(CbltError::KdlParseError {
                details: format!("No directives specified for host {}", hostname),
            });
        }

        if hosts.contains_key(&hostname) {
            return Err(CbltError::KdlParseError {
                details: format!("Host '{}' already exists", hostname),
            });
        }
        hosts.insert(hostname, directives);
    }

    #[cfg(debug_assertions)]
    debug!("{:#?}", hosts);
    Ok(hosts)
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn get_string_args<'a>(node: &'a KdlNode) -> Vec<&'a str> {
    node.entries()
        .iter()
        .filter_map(|e| e.value().as_string())
        .collect::<Vec<&'a str>>()
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
fn parse_reverse_proxy_options(node: &KdlNode) -> Result<ReverseProxyOptions, CbltError> {
    let mut options = ReverseProxyOptions {
        lb_retries: 2,
        lb_interval: 60,
        lb_timeout: 1,
        lb_policy: Some(LoadBalancePolicy::RoundRobin),
    };

    if let Some(children) = node.children() {
        for child in children.nodes() {
            let name = child.name().value();
            match name {
                "lb_retries" => {
                    let args = get_string_args(child);
                    if let Some(retries) = args.first() {
                        options.lb_retries = retries.parse()?;
                    } else {
                        options.lb_retries = 2;
                    }
                }
                "lb_interval" => {
                    let args = get_string_args(child);
                    if let Some(interval) = args.first() {
                        options.lb_interval = interval.parse::<humantime::Duration>()?.as_secs();
                    } else {
                        options.lb_interval = 10;
                    }
                }
                "lb_timeout" => {
                    let args = get_string_args(child);
                    if let Some(timeout) = args.first() {
                        options.lb_timeout = timeout.parse::<humantime::Duration>()?.as_secs();
                    } else {
                        options.lb_timeout = 1;
                    }
                }
                "lb_policy" => {
                    let args = get_string_args(child);
                    if let Some(policy_name) = args.first() {
                        match *policy_name {
                            "round_robin" => {
                                options.lb_policy = Some(LoadBalancePolicy::RoundRobin);
                            }
                            "ip_hash" => {
                                options.lb_policy = Some(LoadBalancePolicy::IPHash);
                            }
                            _ => {
                                return Err(CbltError::KdlParseError {
                                    details: format!("Unknown lb_policy '{}'", policy_name),
                                });
                            }
                        }
                    }
                }
                _ => {
                    return Err(CbltError::KdlParseError {
                        details: format!("Unknown reverse_proxy option '{}'", name),
                    });
                }
            }
        }
    }

    Ok(options)
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub async fn load_servers_from_config(args: Arc<Args>) -> Result<HashMap<u16, Server>, CbltError> {
    let cbltfile_content = fs::read_to_string(&args.cfg).await?;
    let doc: KdlDocument = cbltfile_content.parse()?;
    let config = build_config(&doc)?;

    build_servers(config)
}

#[cfg_attr(feature = "trace", instrument(level = "trace", skip_all))]
pub async fn load_servers_from_docker(_args: Arc<Args>) -> Result<HashMap<u16, Server>, CbltError> {
    use bollard::Docker;
    let docker = Docker::connect_with_local_defaults()?;
    use std::default::Default;

    let options = Some(ListServicesOptions::<String> {
        filters: HashMap::new(),
        ..Default::default()
    });

    let services = docker.list_services(options).await?;

    // Map to hold the directives per host
    let mut hosts: HashMap<String, Vec<Directive>> = HashMap::new();
    for service in services {
        // Process each service
        if let Some(spec) = service.spec {
            if let Some(labels) = spec.labels {
                // Check if labels have keys starting with "cblt."
                if labels.keys().any(|k| k.starts_with("cblt.")) {
                    // Get service name
                    let service_name = spec.name.ok_or(CbltError::ServiceNameNotFound)?;
                    let mut destinations: Vec<String> = Vec::new();
                    let containers = docker
                        .list_containers(Some(ListContainersOptions::<String> {
                            all: false,
                            filters: HashMap::new(),
                            ..Default::default()
                        }))
                        .await?;
                    for container in &containers {
                        if let Some(names) = &container.names {
                            match names
                                .iter()
                                .find(|name| name.starts_with(&format!("/{}.", service_name)))
                            {
                                None => {}
                                Some(name_all) => {
                                    let container_name = name_all.replace("/", "");
                                    debug!("{container_name}");
                                    destinations.push(container_name);
                                }
                            }
                        } else {
                            return Err(CbltError::ContainerNameNotFound);
                        }
                    }

                    // Process the labels
                    let hosts_label =
                        labels
                            .get("cblt.hosts")
                            .ok_or_else(|| CbltError::LabelNotFound {
                                details: "cblt.hosts".to_string(),
                            })?;
                    let path_label =
                        labels
                            .get("cblt.path")
                            .ok_or_else(|| CbltError::LabelNotFound {
                                details: "cblt.path".to_string(),
                            })?;
                    let port_label =
                        labels
                            .get("cblt.port")
                            .ok_or_else(|| CbltError::LabelNotFound {
                                details: "cblt.port".to_string(),
                            })?;

                    let hosts_list: Vec<&str> = hosts_label.split(',').map(|s| s.trim()).collect();
                    let path = path_label.clone();
                    let port =
                        port_label
                            .parse::<u16>()
                            .map_err(|_| CbltError::InvalidLabelFormat {
                                details: "cblt.port".to_string(),
                            })?;

                    // Collect secrets per host
                    let secrets_label = labels.get("cblt.secrets");
                    let secrets_map = if let Some(secrets_label) = secrets_label {
                        let mut map = HashMap::new();
                        let secrets_entries: Vec<&str> =
                            secrets_label.split(',').map(|s| s.trim()).collect();
                        for entry in secrets_entries {
                            let parts: Vec<&str> = entry.split_whitespace().collect();
                            if parts.len() == 3 {
                                let host = parts[0];
                                let key = parts[1].to_string();
                                let cert = parts[2].to_string();
                                map.insert(host.to_string(), (key, cert));
                            } else {
                                return Err(CbltError::InvalidLabelFormat {
                                    details: "cblt.secrets".to_string(),
                                });
                            }
                        }
                        map
                    } else {
                        HashMap::new()
                    };

                    // Load balancing options
                    let lb_policy_label = labels.get("cblt.lb_policy");
                    let lb_interval_label = labels.get("cblt.lb_interval");
                    let lb_timeout_label = labels.get("cblt.lb_timeout");
                    let lb_retries_label = labels.get("cblt.lb_retries");

                    let lb_policy = if let Some(policy_str) = lb_policy_label {
                        match policy_str.as_str() {
                            "round_robin" => Some(LoadBalancePolicy::RoundRobin),
                            "ip_hash" => Some(LoadBalancePolicy::IPHash),
                            _ => {
                                return Err(CbltError::KdlParseError {
                                    details: format!("Unknown lb_policy '{}'", policy_str),
                                });
                            }
                        }
                    } else {
                        None
                    };

                    let lb_interval = if let Some(interval_str) = lb_interval_label {
                        humantime::parse_duration(interval_str)
                            .map_err(|_| CbltError::InvalidLabelFormat {
                                details: "cblt.lb_interval".to_string(),
                            })?
                            .as_secs()
                    } else {
                        10 // Default value
                    };

                    let lb_timeout = if let Some(timeout_str) = lb_timeout_label {
                        humantime::parse_duration(timeout_str)
                            .map_err(|_| CbltError::InvalidLabelFormat {
                                details: "cblt.lb_timeout".to_string(),
                            })?
                            .as_secs()
                    } else {
                        1 // Default value
                    };

                    let lb_retries = if let Some(retries_str) = lb_retries_label {
                        retries_str
                            .parse::<u64>()
                            .map_err(|_| CbltError::InvalidLabelFormat {
                                details: "cblt.lb_retries".to_string(),
                            })?
                    } else {
                        2 // Default value
                    };

                    let options = ReverseProxyOptions {
                        lb_retries,
                        lb_interval,
                        lb_timeout,
                        lb_policy,
                    };

                    // Build the ReverseProxy directive
                    let destinations = destinations
                        .iter()
                        .map(|s| format!("{}:{}", s, port))
                        .collect();
                    let reverse_proxy_directive = Directive::ReverseProxy {
                        pattern: path.clone(),
                        destinations,
                        options,
                    };

                    // For each host, add the directives
                    for host in hosts_list {
                        let host_directives = hosts.entry(host.to_string()).or_default();
                        host_directives.push(reverse_proxy_directive.clone());

                        // If there is a secret for this host, add the TLS directive
                        if let Some((key, cert)) = secrets_map.get(host) {
                            let key_data = Some(key.into());
                            let cert_data = Some(cert.into());
                            host_directives.push(Directive::TlS {
                                key: key_data.ok_or(CbltError::SecretDataNotFound)?,
                                cert: cert_data.ok_or(CbltError::SecretDataNotFound)?,
                            });
                        }
                    }
                }
            }
        }
    }

    // Now we have hosts HashMap<String, Vec<Directive>>
    // We can now build the servers
    build_servers(hosts)
}

#[cfg(test)]
mod tests {
    use crate::config::build_config;
    use kdl::KdlDocument;
    use std::error::Error;

    #[test]
    fn test_simple() -> Result<(), Box<dyn Error>> {
        let cblt_file = r#"
example.com {
    root "*" "/path/to/folder"
    file_server
}
            "#;
        let doc: KdlDocument = cblt_file.parse()?;
        let config = build_config(&doc)?;
        println!("{:#?}", config);

        Ok(())
    }

    #[test]
    fn test_complicated() -> Result<(), Box<dyn Error>> {
        let cblt_file = r#"
example1.com {
    root "*" "/path/folder"
    file_server
    reverse_proxy "/api/*" "localhost:8080"
}

"http://example1.com" {
    redir "https://example2.com{uri}"
}
            "#;
        let doc: KdlDocument = cblt_file.parse()?;
        let config = build_config(&doc)?;
        println!("{:#?}", config);

        Ok(())
    }

    #[test]
    fn test_tls() -> Result<(), Box<dyn Error>> {
        let cblt_file = r#"
example.com {
    root "*" "/path/to/folder"
    file_server
    tls "/path/to/your/certificate.crt" "/path/to/your/private.key"
}
            "#;
        let doc: KdlDocument = cblt_file.parse()?;
        let config = build_config(&doc)?;
        println!("{:#?}", config);

        Ok(())
    }

    #[test]
    fn test_reverse_proxy_with_options() -> Result<(), Box<dyn Error>> {
        let cblt_file = r#"
"example.com" {
    reverse_proxy "/api/*" "backend1:8080" "backend2:8080" {
        health_uri "/health"
        health_interval "10s"
        health_timeout "2s"
        lb_policy "round_robin"
    }
}
            "#;
        let doc: KdlDocument = cblt_file.parse()?;
        let config = build_config(&doc)?;
        println!("{:#?}", config);

        Ok(())
    }

    #[test]
    fn test_reverse_proxy_with_cookie_lb_policy() -> Result<(), Box<dyn Error>> {
        let cblt_file = r#"
"example.com" {
    reverse_proxy "/api/*" "backend1:8080" "backend2:8080" {
        lb_policy "cookie" {
            lb_cookie_name "my_sticky_cookie"
            lb_cookie_path "/"
            lb_cookie_max_age "3600"
        }
    }
}
            "#;
        let doc: KdlDocument = cblt_file.parse()?;
        let config = build_config(&doc)?;
        println!("{:#?}", config);

        Ok(())
    }
}
