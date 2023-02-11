use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_yaml::Value;

mod dockerfile;
use dockerfile::DockerFile;

#[derive(Debug)]
struct DockerContainer {
    name: String,
    build_dir: String,
}

impl DockerContainer {
    fn from_docker_file(file: DockerFile) -> Vec<DockerContainer> {
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

fn header(msg: &str) {
    println!("\x1b[45;37;1m{}\x1b[0m", msg);
}

fn git_version() -> Result<String> {
    let date = Command::new("git")
        .arg("log")
        .arg("-1")
        .arg("--format=%as")
        .output()?
        .stdout;
    let date = String::from_utf8(date)?.trim().to_string();
    let version = Command::new("git")
        .arg("describe")
        .arg("--always")
        .arg("--dirty")
        .output()?
        .stdout;
    let version = String::from_utf8(version)?.trim().to_string();
    Ok(format!("{}-{}", date, version))
}

#[derive(Debug)]
struct BuildContext {
    registry: String,
    version: String,
    config: DepConfig,
    pull: bool,
    containers: Vec<DockerContainer>,
}

impl BuildContext {
    fn new(
        version: String,
        config: DepConfig,
        pull: bool,
        containers: Vec<DockerContainer>,
    ) -> Self {
        BuildContext {
            registry: config.registry.clone(),
            version,
            config,
            pull,
            containers,
        }
    }

    fn transform_docker_compose(&self) -> Result<String> {
        let input_text = std::fs::read_to_string("docker-compose.yaml")?;
        let mut input: Value = serde_yaml::from_str(&input_text)?;
        let services = input
            .get_mut("services")
            .and_then(|k| k.as_mapping_mut())
            .context("No services in docker-compose")?;

        for (service_name, service) in services.iter_mut() {
            let build = service.get("build");
            if let Some(Value::String(_)) = build {
                if let Value::String(service_name) = service_name {
                    let service = service.as_mapping_mut().context("service is not a map")?;
                    let container: Vec<_> = self
                        .containers
                        .iter()
                        .filter(|c| &c.name == service_name)
                        .collect();
                    let container = container[0];
                    service.insert(
                        Value::String("image".to_string()),
                        Value::String(self.image(container)),
                    );
                    service.remove(Value::String("build".into()));
                }
            }
        }

        let output = serde_yaml::to_string(&input)?;

        Ok(output)
    }

    fn run_build_script(&self) -> Result<()> {
        let prefix = r"
set -o errexit
set -o nounset
set -o pipefail";
        if let Some(build_script) = &self.config.build {
            let script = format!("{}\n{}", prefix, build_script);
            println!("Executing\x1b[48;2;10;10;10m\n{}\x1b[0m", script);
            let mut process = Command::new("bash").stdin(Stdio::piped()).spawn()?;
            let stdin = process.stdin.as_mut().context("No stdin")?;
            writeln!(stdin, "{}", script)?;
            if !process.wait()?.success() {
                bail!("Failed to execut build script");
            }
        }
        Ok(())
    }

    fn build_all(&self) -> Result<()> {
        self.run_build_script()?;
        for container in self.containers.iter() {
            self.build(container)?;
            println!();
        }
        Ok(())
    }

    fn deploy(&self) -> Result<()> {
        self.push()?;
        header("Deploying");
        if self.pull {
            let status = Command::new("ssh")
                .arg(&self.config.server)
                .arg(format!("cd {} && docker compose pull", self.config.name))
                .status()?;
            if !status.success() {
                bail!("Failed to docker compose pull");
            }
        }
        let status = Command::new("ssh")
            .arg(&self.config.server)
            .arg(format!("cd {} && docker compose up -d", self.config.name))
            .status()?;
        if !status.success() {
            bail!("Failed to run docker compose up -d");
        }

        Ok(())
    }

    fn push(&self) -> Result<()> {
        self.push_containers()?;
        self.push_files()?;
        Ok(())
    }

    fn push_containers(&self) -> Result<()> {
        self.build_all()?;
        for container in self.containers.iter() {
            let status = Command::new("docker")
                .arg("push")
                .arg(self.image(container))
                .status()?;
            if !status.success() {
                bail!("Failed to push container {}", container.name);
            }
        }

        Ok(())
    }

    fn remote_dir(&self) -> String {
        format!("{}:{}", self.config.server, self.config.name)
    }

    fn push_files(&self) -> Result<()> {
        let tmp_dir = tempfile::tempdir()?;
        let compose_txt = self.transform_docker_compose()?;
        let mut tmp_file_path = tmp_dir.path().to_owned();
        tmp_file_path.push("docker-compose.yaml");
        std::fs::write(tmp_file_path, compose_txt)?;

        // tmp_dir_path must have a trailing slash.
        let tmp_dir_path = format!("{}/", tmp_dir.path().display());
        let mut all_paths: Vec<String> = vec![tmp_dir_path];
        let additional_files = match &self.config.additional_files {
            Some(files) => files.clone(),
            None => vec![],
        };

        for add in additional_files.into_iter() {
            all_paths.push(add.display().to_string());
        }

        let mut proc = Command::new("rsync");
        proc.arg("--verbose")
            .arg("--archive")
            .arg("-h")
            .arg("--progress")
            .args(all_paths)
            .arg(self.remote_dir());

        match proc.status()?.success() {
            true => Ok(()),
            false => bail!("Failed to push rsync"),
        }
    }

    fn build(&self, container: &DockerContainer) -> Result<()> {
        header(&format!("Building {}", self.image(container)));
        let mut builder = Command::new("docker");
        builder.arg("build");
        if self.pull {
            builder.arg("--pull");
        }
        builder.arg(&container.build_dir);
        builder.arg("-t").arg(self.image(container));

        let status = builder.status()?;
        if !status.success() {
            bail!("Failed to execute docker build")
        }
        Ok(())
    }

    fn image(&self, c: &DockerContainer) -> String {
        format!("{}/{}:{}", self.registry, c.name, self.version)
    }
}

#[derive(Deserialize, Debug)]
struct DepConfig {
    server: String,
    registry: String,
    name: String,
    #[serde(alias = "additionalFiles")]
    additional_files: Option<Vec<PathBuf>>,
    build: Option<String>,
}

#[derive(Parser)]
#[command(author, version, about, long_about=None)]
struct Cli {
    /// Run docker image pull before building and deploying.
    #[arg(short, long, value_name = "PULL")]
    pull: bool,

    /// Directory to change into before running the commands
    #[arg(short, long)]
    directory: Option<PathBuf>,

    #[command(subcommand)]
    command: CliCommand,
}

#[derive(clap::Subcommand)]
enum CliCommand {
    /// Build.
    Build,
    /// Build, and push to the server, but don't deploy.
    Push,
    /// Build, push, and deploy to the server.
    Deploy,
    /// Display git version and exit.
    Version,
    /// Print the generated docker-compose file
    Compose,
}

fn read_docker_compose() -> Result<Vec<DockerContainer>> {
    let docker_path = Path::new("docker-compose.yaml");
    let open = File::open(docker_path).context("Failed to open docker-compose.yaml")?;

    let docker_file: DockerFile =
        serde_yaml::from_reader(open).context("Failed to parse docker-compose.yaml")?;

    Ok(DockerContainer::from_docker_file(docker_file))
}

fn read_dep() -> Result<DepConfig> {
    let path = Path::new(".dep.yaml");
    let open = File::open(path).context("Failde to open .dep.yaml")?;
    let deserialized: DepConfig =
        serde_yaml::from_reader(open).context("Failed to parse .dep.yaml")?;
    Ok(deserialized)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(dir) = &cli.directory {
        std::env::set_current_dir(dir).context("Failed to change directory")?;
    }

    let containers = read_docker_compose()?;
    let dep = read_dep()?;

    let build_context = BuildContext::new(git_version()?, dep, cli.pull, containers);

    match cli.command {
        CliCommand::Version => {
            println!("version: {}", git_version()?);
        }
        CliCommand::Build => build_context.build_all()?,
        CliCommand::Push => build_context.push()?,
        CliCommand::Compose => {
            let output = build_context.transform_docker_compose()?;
            println!("{}", output);
        }
        CliCommand::Deploy {} => build_context.deploy()?,
    }

    Ok(())
}
