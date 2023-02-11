use std::io::{self, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct DepConfig {
    pub name: String,
    pub server: String,
    pub registry: String,
    #[serde(rename = "additionalFiles")]
    pub additional_files: Option<Vec<PathBuf>>,
    pub build: Option<String>,
}

impl DepConfig {
    pub fn create_interactive() -> Self {
        let current_directory_default: Option<String> = match std::env::current_dir() {
            Ok(pathbuf) => pathbuf
                .file_name()
                .and_then(|x| x.to_str())
                .map(|s| s.to_string()),
            Err(_) => None,
        };

        Self {
            name: ask("What the name of this program?", current_directory_default),
            server: ask("What is the name of the server?", None),
            registry: ask("What is URL of the docker registry?", None),
            additional_files: Some(vec![]),
            build: Some("".to_string()),
        }
    }
}

fn ask(question: &str, default: Option<String>) -> String {
    print!("{question} ");
    if let Some(default) = &default {
        print!("({}): ", default);
    }

    std::io::stdout().flush().unwrap();
    let stdin = io::stdin();
    let mut buf = String::new();
    loop {
        match stdin.read_line(&mut buf) {
            Err(e) => {
                println!("\x1b[31merror\x1b[0m: {}", e);
                continue;
            }
            Ok(_) => (),
        }
        buf = buf.trim().to_string();
        if !buf.is_empty() {
            return buf;
        } else if let Some(default) = default {
            return default.clone();
        }
    }
}
