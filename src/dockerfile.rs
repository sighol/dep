use std::collections::HashMap;

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct DockerFile {
    pub services: HashMap<String, DockerService>,
}

#[derive(Deserialize, Debug)]
pub struct DockerService {
    pub build: Option<DockerBuild>,
    pub restart: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum DockerBuild {
    Str(String),
    Advanced(DockerBuildAdvanced),
}

#[derive(Deserialize, Debug)]
pub struct DockerBuildAdvanced {
    pub context: String,
    pub dockerfile: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug)]
pub struct DockerContainer {
    pub name: String,
    pub build_dir: String,
    pub dockerfile: Option<String>,
    pub target: Option<String>,
}

impl DockerContainer {
    pub fn from_docker_file(file: DockerFile) -> Vec<DockerContainer> {
        let mut output = vec![];
        for (service_name, service) in file.services.into_iter() {
            match service.restart {
                None => eprintln!(
                    "\x1b[41;1mError\x1b[0m Service {service_name} does not have a `restart` property set"
                ),
                Some(x) => {
                    if x != "unless-stopped" && x != "always" {
                        eprintln!("\x1b[41;1mError\x1b[0m Service {service_name} has `restart` set to `{x}`")
                    }
                }
            };
            if let Some(build_dir) = service.build {
                if let DockerBuild::Str(s) = build_dir {
                    output.push(DockerContainer {
                        name: service_name,
                        build_dir: s,
                        dockerfile: None,
                        target: None,
                    })
                } else if let DockerBuild::Advanced(adv) = build_dir {
                    output.push(DockerContainer {
                        name: service_name,
                        build_dir: adv.context,
                        dockerfile: adv.dockerfile,
                        target: adv.target,
                    })
                }
            }
        }
        output.sort_by_key(|k| k.name.clone());
        output
    }
}
