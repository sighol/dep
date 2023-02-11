use std::collections::HashMap;

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct DockerFile {
    pub services: HashMap<String, DockerService>,
}

#[derive(Deserialize, Debug)]
pub struct DockerService {
    pub build: Option<String>,
}

#[derive(Debug)]
pub struct DockerContainer {
    pub name: String,
    pub build_dir: String,
}

impl DockerContainer {
    pub fn from_docker_file(file: DockerFile) -> Vec<DockerContainer> {
        let mut output = vec![];
        for (service_name, service) in file.services.into_iter() {
            if let Some(build_dir) = service.build {
                output.push(DockerContainer {
                    name: service_name,
                    build_dir,
                })
            }
        }
        output.sort_by_key(|k| k.name.clone());
        output
    }
}
