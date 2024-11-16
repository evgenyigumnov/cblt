use crate::error::CbltError;
use kdl::KdlDocument;
use log::debug;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Directive {
    Root {
        pattern: heapless::String<200>,
        path: heapless::String<200>,
    },
    FileServer,
    ReverseProxy {
        pattern: heapless::String<200>,
        destination: heapless::String<200>,
    },
    Redir {
        destination: heapless::String<200>,
    },
    TlS {
        cert: heapless::String<200>,
        key: heapless::String<200>,
    },
}

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
                            let pattern = args
                                .get(0)
                                .ok_or(CbltError::KdlParseError {
                                    details: "pattern absent".to_string(),
                                })?
                                .to_string();
                            let path = args
                                .get(1)
                                .ok_or(CbltError::KdlParseError {
                                    details: "path absent".to_string(),
                                })?
                                .to_string();
                            directives.push(Directive::Root {
                                pattern: heapless::String::try_from(pattern.as_str())
                                    .map_err(|_| CbltError::HeapLessError {})?,
                                path: heapless::String::try_from(path.as_str())
                                    .map_err(|_| CbltError::HeapLessError {})?,
                            });
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
                            let pattern = args
                                .get(0)
                                .ok_or(CbltError::KdlParseError {
                                    details: "pattern absent".to_string(),
                                })?
                                .to_string();
                            let destination = args
                                .get(1)
                                .ok_or(CbltError::KdlParseError {
                                    details: "destination absent".to_string(),
                                })?
                                .to_string();
                            directives.push(Directive::ReverseProxy {
                                pattern: heapless::String::try_from(pattern.as_str())
                                    .map_err(|_| CbltError::HeapLessError {})?,
                                destination: heapless::String::try_from(destination.as_str())
                                    .map_err(|_| CbltError::HeapLessError {})?,
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
                        if args.len() >= 1 {
                            let destination = args
                                .get(0)
                                .ok_or(CbltError::KdlParseError {
                                    details: "destination absent".to_string(),
                                })?
                                .to_string();
                            directives.push(Directive::Redir {
                                destination: heapless::String::try_from(destination.as_str())
                                    .map_err(|_| CbltError::HeapLessError {})?,
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
                            let cert_path = args
                                .get(0)
                                .ok_or(CbltError::KdlParseError {
                                    details: "cert path absent".to_string(),
                                })?
                                .to_string();
                            let key_path = args
                                .get(1)
                                .ok_or(CbltError::KdlParseError {
                                    details: "key path absent".to_string(),
                                })?
                                .to_string();
                            directives.push(Directive::TlS {
                                cert: heapless::String::try_from(cert_path.as_str())
                                    .map_err(|_| CbltError::HeapLessError {})?,
                                key: heapless::String::try_from(key_path.as_str())
                                    .map_err(|_| CbltError::HeapLessError {})?,
                            });
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

fn get_string_args<'a>(node: &'a kdl::KdlNode) -> Vec<&'a str> {
    node.entries()
        .iter()
        .filter_map(|e| e.value().as_string())
        .collect::<Vec<&'a str>>()
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
}
