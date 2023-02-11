# dep

`dep` is a small deployment cli based on docker-compose. It builds all
container images locally, pushes the container images to a registry, and runs
`docker compose up -d` to start/restart the containers. This means that
deployments will include some downtime.

# Installation

```shell
cargo install --git https://github.com/sighol/dep
```

# Usage

Run `dep init` in a folder that contains `docker-compose.yaml` to create a
`deployment.yaml` file. You can then run `dep deploy` to deploy your
application.

```
Usage: dep [OPTIONS] <COMMAND>

Commands:
  build    Build
  push     Build and push to the server
  deploy   Build, push, and deploy to the server
  version  Display git version
  compose  Display the generated docker-compose.yaml file
  init     Interactive wizard to create a deployment.yaml file
  help     Print this message or the help of the given subcommand(s)

Options:
  -p, --pull                   Run docker image pull before building and deploying
  -d, --directory <DIRECTORY>  Directory to change into before running the commands
  -h, --help                   Print help
  -V, --version                Print version
```

# Example

docker-compose.yaml:

```yaml
version: "3"
services:
  web-server:
    build: .
    environment:
      GIN_MODE: "debug"
    volumes:
      - "./ui:/app/ui"
    # restart should be set to either `restart` or `unless-stopped`.
    restart: always
    ports:
      - "127.0.0.1:1339:8080"
```

deployment.yaml

```yaml
name: example-service
server: example.org
registry: https://registry.example.org
additionalFiles: []
build: ""
```

If you run `dep deploy`, it will

- First run the contents of `build` as a bash script.
- Then try to build all services in `docker-compose.yaml` that contains a `build: ` field.
  The services will be tagged with the registry, the service name, the date and git revision.
  In the example above, it might build and tag `https://registry.example.org/example-service/web-server:2023-01-30-90348ce`.
- Create a temporary `docker-compose.yaml`file where the `build: .` fields have been replaced by the `image: TAG`.
- rsync the generated `docker-compose.yaml` and any additional files listed in `additional_files`.
- Push the generated images to the docker registry.
- ssh into the server and run `docker compose up -d`.
