use anyhow::{bail, Context, Result};
use clap::Parser;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_yaml::Value;

mod dockerfile;
use dockerfile::{DockerContainer, DockerFile};

mod config;
use config::DepConfig;

const DOCKER_COMPOSE_PATH: &str = "docker-compose.yaml";
const DEP_CONFIG_PATH: &str = "deployment.yaml";

fn header(msg: &str) {
    println!("\x1b[45;37;1m{}\x1b[0m", msg);
}

fn header_elapsed(msg: &str, instant: &Instant) {
    println!(
        "\x1b[45;37;1m{} in {:.2} seconds\x1b[0m",
        msg,
        instant.elapsed().as_secs_f64()
    );
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
        let input_text = std::fs::read_to_string(DOCKER_COMPOSE_PATH)?;
        let mut input: Value = serde_yaml::from_str(&input_text)?;
        let services = input
            .get_mut("services")
            .and_then(|k| k.as_mapping_mut())
            .context("No services in docker-compose")?;

        for (service_name, service) in services.iter_mut() {
            let build = service.get("build");
            if let Some(_) = build {
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
            header("Running build script");
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
        let start = Instant::now();
        for container in self.containers.iter() {
            self.build(container)?;
            println!();
        }
        header_elapsed("Built all containers", &start);
        Ok(())
    }

    fn deploy(&self) -> Result<()> {
        let start = Instant::now();
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
        header_elapsed("Deployed", &start);

        Ok(())
    }

    fn push(&self) -> Result<()> {
        let start = Instant::now();
        self.push_containers()?;
        self.push_files()?;
        header_elapsed("Pushed everything", &start);
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
        tmp_file_path.push(DOCKER_COMPOSE_PATH);
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
        builder.arg("--build-arg").arg(format!("VERSION={}", &self.version));
        if self.pull {
            builder.arg("--pull");
        }
        builder.arg(&container.build_dir);
        if let Some(file) = &container.dockerfile {
            builder.arg("-f").arg(file.to_string());
        }
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

#[derive(Parser)]
#[command(author, version, about, long_about=None)]
struct Cli {
    /// Run docker image pull before building and deploying.
    #[arg(global = true, short, long, value_name = "PULL")]
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
    /// Build and push to the server.
    Push {
        #[arg(short, long)]
        no_docker: bool,
    },
    /// Build, push, and deploy to the server.
    Deploy,
    /// Display git version.
    Version,
    /// Display the generated docker-compose.yaml file.
    Compose,
    /// Interactive wizard to create a deployment.yaml file.
    Init,
}

fn read_docker_compose() -> Result<Vec<DockerContainer>> {
    let docker_path = Path::new(DOCKER_COMPOSE_PATH);
    let open =
        File::open(docker_path).context(format!("Failed to open {}", DOCKER_COMPOSE_PATH))?;

    let docker_file: DockerFile = serde_yaml::from_reader(open)
        .context(format!("Failed to parse {}", DOCKER_COMPOSE_PATH))?;

    Ok(DockerContainer::from_docker_file(docker_file))
}

fn read_dep() -> Result<DepConfig> {
    let path = Path::new(DEP_CONFIG_PATH);
    let open =
        File::open(path).context(format!("Failed to open config file: {}", DEP_CONFIG_PATH))?;
    let deserialized: DepConfig = serde_yaml::from_reader(open)
        .context(format!("Failed to parse config file: {}", DEP_CONFIG_PATH))?;
    Ok(deserialized)
}

fn init() -> Result<()> {
    let dep_path = Path::new(DEP_CONFIG_PATH);
    if Path::exists(&dep_path) {
        print!(
            "{} already exists. Are you sure you want to overwrite it? (y/n) ",
            DEP_CONFIG_PATH
        );
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        answer = answer.trim_end().to_string();
        if answer != "y" && answer != "yes" {
            return Ok(());
        }
    }
    let config = DepConfig::create_interactive();
    let mut write_handle = File::create(dep_path)?;
    serde_yaml::to_writer(&mut write_handle, &config)?;
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(dir) = &cli.directory {
        std::env::set_current_dir(dir)
            .context(format!("Failed to change directory to {}", dir.display()))?;
    }

    if let CliCommand::Init = &cli.command {
        init()?;
        std::process::exit(0);
    }

    let containers = read_docker_compose()?;
    let dep = read_dep()?;

    let build_context = BuildContext::new(git_version()?, dep, cli.pull, containers);

    match cli.command {
        CliCommand::Version => {
            println!("version: {}", git_version()?);
        }
        CliCommand::Build => build_context.build_all()?,
        CliCommand::Push { no_docker } => match no_docker {
            true => build_context.push_files()?,
            false => build_context.push()?,
        },
        CliCommand::Compose => {
            let output = build_context.transform_docker_compose()?;
            println!("{}", output);
        }
        CliCommand::Deploy => build_context.deploy()?,
        CliCommand::Init => {}
    }

    Ok(())
}
