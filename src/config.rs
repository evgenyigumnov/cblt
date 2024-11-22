use crate::error::CbltError;
use kdl::{KdlDocument, KdlNode};
use log::debug;
use std::collections::HashMap;
#[cfg(feature = "trace")]
use tracing::instrument;

#[derive(Debug, Clone)]
pub enum Directive {
    Root {
        pattern: String,
        path: String,
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
    pub health_uri: Option<String>,
    pub health_interval: Option<String>,
    pub health_timeout: Option<String>,
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
                            directives.push(Directive::Root { pattern, path });
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

fn get_string_args<'a>(node: &'a KdlNode) -> Vec<&'a str> {
    node.entries()
        .iter()
        .filter_map(|e| e.value().as_string())
        .collect::<Vec<&'a str>>()
}

fn parse_reverse_proxy_options(node: &KdlNode) -> Result<ReverseProxyOptions, CbltError> {
    let mut options = ReverseProxyOptions::default();

    if let Some(children) = node.children() {
        for child in children.nodes() {
            let name = child.name().value();
            match name {
                "health_uri" => {
                    let args = get_string_args(child);
                    if let Some(uri) = args.get(0) {
                        options.health_uri = Some((*uri).to_string());
                    }
                }
                "health_interval" => {
                    let args = get_string_args(child);
                    if let Some(interval) = args.get(0) {
                        options.health_interval = Some((*interval).to_string());
                    }
                }
                "health_timeout" => {
                    let args = get_string_args(child);
                    if let Some(timeout) = args.get(0) {
                        options.health_timeout = Some((*timeout).to_string());
                    }
                }
                "lb_policy" => {
                    let args = get_string_args(child);
                    if let Some(policy_name) = args.get(0) {
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
