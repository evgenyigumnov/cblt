use kdl::KdlDocument;
use log::debug;
use std::collections::HashMap;
use std::error::Error;

#[derive(Debug)]
pub struct Config {
    pub hosts: HashMap<String, HostConfig>,
}

#[derive(Debug)]
pub struct HostConfig {
    pub directives: Vec<Directive>,
}

#[derive(Debug)]
pub enum Directive {
    Root {
        pattern: String,
        path: String,
    },
    FileServer,
    ReverseProxy {
        pattern: String,
        destination: String,
    },
    Redir {
        destination: String,
    },
}

pub fn build_config(doc: &KdlDocument) -> Result<Config, Box<dyn Error>> {
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
                            let pattern = args.get(0).unwrap().to_string();
                            let path = args.get(1).unwrap().to_string();
                            directives.push(Directive::Root { pattern, path });
                        } else {
                            return Err(
                                format!("Invalid 'root' directive for host {}", hostname).into()
                            );
                        }
                    }
                    "file_server" => {
                        directives.push(Directive::FileServer);
                    }
                    "reverse_proxy" => {
                        let args = get_string_args(child_node);
                        if args.len() >= 2 {
                            let pattern = args.get(0).unwrap().to_string();
                            let destination = args.get(1).unwrap().to_string();
                            directives.push(Directive::ReverseProxy {
                                pattern,
                                destination,
                            });
                        } else {
                            return Err(format!(
                                "Invalid 'reverse_proxy' directive for host {}",
                                hostname
                            )
                            .into());
                        }
                    }
                    "redir" => {
                        let args = get_string_args(child_node);
                        if args.len() >= 1 {
                            let destination = args.get(0).unwrap().to_string();
                            directives.push(Directive::Redir { destination });
                        } else {
                            return Err(
                                format!("Invalid 'redir' directive for host {}", hostname).into()
                            );
                        }
                    }
                    _ => {
                        return Err(format!(
                            "Unknown directive '{}' for host {}",
                            child_name, hostname
                        )
                        .into());
                    }
                }
            }
        }

        if directives.is_empty() {
            return Err(format!("No directives specified for host {}", hostname).into());
        }

        hosts.insert(hostname, HostConfig { directives });
    }

    let ret = Config { hosts };
    #[cfg(debug_assertions)]
    debug!("{:#?}", ret);
    Ok(ret)
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
}
