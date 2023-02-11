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
