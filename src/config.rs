use std::collections::HashMap;
use std::error::Error;
use kdl::KdlDocument;

pub struct Config {
    pub hosts: HashMap<String, HostConfig>,
}

pub struct HostConfig {
    pub root: String,
    pub pattern: String,
}

pub fn build_config(doc: &KdlDocument) -> Result<Config, Box<dyn Error>> {

    let mut hosts = HashMap::new();

    for node in doc.nodes() {
        let hostname = node.name().value().to_string();
        let mut root = String::new();
        let mut pattern = String::new();

        if let Some(children) = node.children() {
            for child_node in children.nodes() {
                let child_name = child_node.name().value();

                if child_name == "root" {
                    let args = child_node
                        .entries()
                        .iter()
                        .filter_map(|e| e.value().as_string())
                        .collect::<Vec<&str>>();
                    if !args.is_empty() {
                        root = args[args.len() - 1].to_string();
                        pattern = args[args.len() - 2].to_string();
                    } else {
                        return Err(format!("No root path specified for host {}", hostname).into());
                    }
                }
            }
        }

        if root.is_empty() {
            return Err(format!("No root specified for host {}", hostname).into());
        }

        hosts.insert(hostname, HostConfig { root, pattern});
    }

    Ok(Config { hosts })
}