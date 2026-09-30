//! The Workers page's reach into the compose project: what the compose file
//! declares, one container's entry to edit, registry versions and search, a
//! look at a directory before a worker points at it, and a trigger type that
//! fires when the daemon writes its state or the compose file changes.
//!
//! Lifecycle stays the daemon's: the page calls `compose::*` itself. The one
//! write here, `console::compose::edit`, merges a settings change with the
//! declared entry and hands the result to `compose::add`.

pub mod declaration;
pub mod entry;
pub mod inspect;
pub mod registry;
pub mod watch;

use std::path::Path;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use declaration::{read_project, ContainerSource, ProjectDeclaration};
use entry::{container_object, read_entry, EditInput};
use inspect::{inspect, InspectInput, Inspection};
use registry::{PackageVersion, RegistryWorker};
use watch::{compose_status, ChangedEvent, ChangedTriggerHandler, ChangedTriggerSpec};

pub const CHANGED_TRIGGER: &str = "console::compose::changed";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ProjectInput {
    /// Compose file on the daemon host; defaults to the daemon project.
    pub file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ContainerInput {
    pub container: String,
    /// Compose file on the daemon host; defaults to the daemon project.
    pub file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct VersionsInput {
    /// A declared package container.
    pub container: Option<String>,
    /// Or a worker name on the default registry, for one not declared yet.
    pub name: Option<String>,
    /// Compose file on the daemon host; defaults to the daemon project.
    pub file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchInput {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct VersionsResult {
    pub container: String,
    pub reference: String,
    /// The selector the compose file declares: an exact version or a channel.
    pub declared: Option<String>,
    /// Newest first, as the registry lists them.
    pub versions: Vec<PackageVersion>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResult {
    pub workers: Vec<RegistryWorker>,
}

/// What `compose::add` answers once it accepts an edit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditAccepted {
    pub operation_id: String,
    pub requested: u64,
    pub status: String,
}

fn handler_error(prefix: &str, error: impl std::fmt::Display) -> Error {
    Error::Handler(format!("{prefix}: {error}"))
}

/// Errors that already carry a `CODE:` prefix pass through unchanged.
fn coded_error(fallback: &str, error: String) -> Error {
    match error.split_once(':') {
        Some((code, _))
            if !code.is_empty() && code.chars().all(|c| c.is_ascii_uppercase() || c == '_') =>
        {
            Error::Handler(error)
        }
        _ => handler_error(fallback, error),
    }
}

async fn compose_file(iii: &IIIClient, file: Option<&str>) -> Result<String, Error> {
    compose_status(iii, file).await?.file.ok_or_else(|| {
        Error::Handler(
            "COMPOSE_UNAVAILABLE: compose::status answered without a compose file".to_string(),
        )
    })
}

async fn read_compose_file(iii: &IIIClient, file: Option<&str>) -> Result<(String, String), Error> {
    let file = compose_file(iii, file).await?;
    let source = tokio::fs::read_to_string(&file)
        .await
        .map_err(|error| handler_error("PROJECT_READ_ERROR", error))?;
    Ok((file, source))
}

/// Register the trigger type, then every `console::compose::*` function.
pub fn register(iii: &Arc<IIIClient>) {
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            CHANGED_TRIGGER,
            "Fires when the compose daemon writes state.json or the compose file changes. Bind with an empty config.",
            ChangedTriggerHandler::new(iii.clone()),
        )
        .trigger_request_format::<ChangedTriggerSpec>()
        .call_request_format::<ChangedEvent>(),
    );

    {
        let iii = iii.clone();
        iii.clone().register_function(
            "console::compose::project",
            RegisterFunction::new_async(move |input: ProjectInput| {
                let iii = iii.clone();
                async move {
                    let file = compose_file(&iii, input.file.as_deref()).await?;
                    read_project(Path::new(&file))
                        .await
                        .map_err(|error| handler_error("PROJECT_READ_ERROR", error))
                }
            })
            .description(
                "Read the compose file as declared: namespace, engine endpoint, timeouts, and each container's source, version, start_after, environment keys and run script.",
            ),
        );
    }

    {
        let iii = iii.clone();
        iii.clone().register_function(
            "console::compose::versions",
            RegisterFunction::new_async(move |input: VersionsInput| {
                let iii = iii.clone();
                async move { versions(&iii, input).await }
            })
            .description(
                "List registry versions, newest first, with their release channels: of a declared package container, or of a worker name on the default registry.",
            ),
        );
    }

    iii.register_function(
        "console::compose::search",
        RegisterFunction::new_async(|input: SearchInput| async move {
            registry::search(&input.query)
                .await
                .map(|workers| SearchResult { workers })
                .map_err(|error| handler_error("REGISTRY_ERROR", error))
        })
        .description(
            "Search the worker registry by name for workers Compose can declare (engine workers are left out).",
        ),
    );

    iii.register_function(
        "console::compose::inspect",
        RegisterFunction::new_async(|input: InspectInput| async move {
            Ok::<Inspection, Error>(inspect(&input).await)
        })
        .description(
            "Inspect a directory on this host: its iii.worker.yaml, whether a relative run command is built, other git checkouts of the same worker, or the workers inside a plain folder.",
        ),
    );

    {
        let iii = iii.clone();
        iii.clone().register_function(
            "console::compose::container",
            RegisterFunction::new_async(move |input: ContainerInput| {
                let iii = iii.clone();
                async move {
                    let (_, source) = read_compose_file(&iii, input.file.as_deref()).await?;
                    read_entry(&source, &input.container)
                        .map_err(|error| coded_error("PROJECT_READ_ERROR", error))
                }
            })
            .description(
                "Read one container's declaration for editing: worker, version, start_after, env_file, environment (credential-like literals masked), run script, and config_override as YAML.",
            ),
        );
    }

    {
        let iii = iii.clone();
        iii.clone().register_function(
            "console::compose::edit",
            RegisterFunction::new_async(move |input: EditInput| {
                let iii = iii.clone();
                async move { edit(&iii, input).await }
            })
            .description(
                "Change one container's worker path, run script, start_after, environment, or config_override. Merges with the declared entry (masked secrets keep their file values) and hands it to compose::add; returns the accepted compose operation.",
            ),
        );
    }
}

async fn versions(iii: &IIIClient, input: VersionsInput) -> Result<VersionsResult, Error> {
    let container_name = match (input.container, input.name) {
        (Some(container), _) => container,
        (None, Some(name)) => {
            let versions = registry::versions(&name)
                .await
                .map_err(|error| handler_error("REGISTRY_ERROR", error))?;
            return Ok(VersionsResult {
                container: name.clone(),
                reference: format!("{}/{name}", registry::DEFAULT_REGISTRY),
                declared: None,
                versions,
            });
        }
        (None, None) => return Err(handler_error("INVALID_INPUT", "pass container or name")),
    };
    let file = compose_file(iii, input.file.as_deref()).await?;
    let declaration: ProjectDeclaration = read_project(Path::new(&file))
        .await
        .map_err(|error| handler_error("PROJECT_READ_ERROR", error))?;
    let container = declaration
        .containers
        .into_iter()
        .find(|c| c.name == container_name)
        .ok_or_else(|| {
            handler_error(
                "UNKNOWN_CONTAINER",
                format!("{container_name} is not declared"),
            )
        })?;
    if container.source != ContainerSource::Package {
        return Err(handler_error(
            "NOT_A_PACKAGE",
            format!("{container_name} runs from a local path"),
        ));
    }
    let versions = registry::versions(&container.worker_ref)
        .await
        .map_err(|error| handler_error("REGISTRY_ERROR", error))?;
    Ok(VersionsResult {
        container: container.name,
        reference: container.worker_ref,
        declared: container.version,
        versions,
    })
}

async fn edit(iii: &IIIClient, input: EditInput) -> Result<EditAccepted, Error> {
    let (file, source) = read_compose_file(iii, input.file.as_deref()).await?;
    let object =
        container_object(&source, &input).map_err(|error| coded_error("INVALID_EDIT", error))?;
    let mut payload = serde_json::json!({ "file": file, "workers": [object] });
    if let Some(operation_id) = &input.operation_id {
        payload["operation_id"] = serde_json::json!(operation_id);
    }
    let accepted = iii
        .trigger(TriggerRequest {
            function_id: "compose::add".to_string(),
            payload,
            action: None,
            timeout_ms: Some(60_000),
        })
        .await?;
    serde_json::from_value(accepted)
        .map_err(|error| handler_error("INVALID_COMPOSE_RESPONSE", error))
}
