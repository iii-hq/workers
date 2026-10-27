# Harness on iii Cloud without provider credentials

Deploy `harness/worker-compose-prod.yaml` with its generated
`harness/worker-compose-prod.lock`. Packages are pinned; only LLM Router is
built from this repository so its configuration initialization uses the
Cloud-supported `configuration::ensure` operation.

Set the environment's build configuration to:

- repository root directory: unset;
- runtime for `llm-router`: `rust@1.89`;
- build command: `cargo build --release --locked --no-default-features --manifest-path llm-router/Cargo.toml`;
- one fixed replica for the application group;
- no release command and no application healthcheck override.

Cloud provisions the State Redis and Queue RabbitMQ resources. Review and
confirm those resources in the deployment plan. It supplies configuration and
observability through its managed engine; do not add those engine workers as
containers. Generate the lock with iii 0.24.6 in CI using
`iii compose build -f harness/worker-compose-prod.yaml`; commit the generated
lock, and validate subsequent builds with `--frozen`.

Provider workers are declared because Harness depends on them, but their API
keys are empty and local OAuth credential imports are disabled. No model
generation is expected to succeed until a provider is configured. Directory
uses lexical search so startup does not download an embedding model.

The public `/mcp` route requires a Cloud API key and keeps
`require_expose: true`. Harness's internal functions are not exposed. An
operator can test registration, session lifecycle and missing-provider errors
through a private connection to this environment. ADE's own UI listener is not
published by this manifest.

## Storage limitation

Cloud currently gives each application pod an ephemeral working directory.
State and Queue resources persist independently, but Session Manager stores
conversations on that filesystem. Conversations, local projects and encrypted
provider secrets can therefore be lost when the pod is replaced. Use
disposable sessions for this deployment test; do not treat it as a durable
production workspace until filesystem persistence is supported.
