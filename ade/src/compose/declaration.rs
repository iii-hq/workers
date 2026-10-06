//! The compose file as declared: namespace, engine endpoint, timeouts, and
//! each container's source, version, `start_after`, environment keys and run
//! script. Values stay in the file; the page reads them through `entry`.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContainerSource {
    Path,
    Package,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeclaredContainer {
    pub name: String,
    pub source: ContainerSource,
    #[serde(rename = "ref")]
    pub worker_ref: String,
    pub version: Option<String>,
    pub start_after: Vec<String>,
    pub environment: Vec<String>,
    pub run: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectDeclaration {
    pub file: String,
    pub namespace: Option<String>,
    pub engine_url: Option<String>,
    pub engine_host: Option<String>,
    pub engine_port: Option<u16>,
    pub startup_timeout: Option<String>,
    pub stop_timeout: Option<String>,
    pub containers: Vec<DeclaredContainer>,
}

fn key(name: &str) -> Value {
    Value::String(name.to_string())
}

fn field<'a>(mapping: &'a Mapping, name: &str) -> Option<&'a Value> {
    mapping.get(key(name))
}

fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect()
}

fn endpoint(url: Option<&str>) -> (Option<String>, Option<u16>) {
    let Some(url) = url else {
        return (None, None);
    };
    let Ok(parsed) = url::Url::parse(url) else {
        return (None, None);
    };
    (parsed.host_str().map(str::to_string), parsed.port())
}

pub fn parse_worker_ref(worker_ref: &str) -> (ContainerSource, String) {
    if let Some(path) = worker_ref.strip_prefix("path://") {
        return (ContainerSource::Path, path.to_string());
    }
    if let Some(package) = worker_ref.strip_prefix("package://") {
        return (ContainerSource::Package, package.to_string());
    }
    (ContainerSource::Unknown, worker_ref.to_string())
}

pub fn parse_project(file: impl Into<String>, source: &str) -> Result<ProjectDeclaration, String> {
    let root: Value = serde_yaml::from_str(source).map_err(|error| error.to_string())?;
    let empty = Mapping::new();
    let root = root.as_mapping().unwrap_or(&empty);
    let engine = field(root, "engine").and_then(Value::as_mapping);
    let engine_url = engine.and_then(|mapping| text(field(mapping, "url")));
    let (engine_host, engine_port) = endpoint(engine_url.as_deref());
    let containers = field(root, "containers")
        .and_then(Value::as_mapping)
        .into_iter()
        .flat_map(Mapping::iter)
        .filter_map(|(name, raw)| {
            let name = name.as_str()?.to_string();
            let empty = Mapping::new();
            let entry = raw.as_mapping().unwrap_or(&empty);
            let (source, worker_ref) =
                parse_worker_ref(text(field(entry, "worker")).as_deref().unwrap_or(""));
            let environment = field(entry, "environment")
                .and_then(Value::as_mapping)
                .into_iter()
                .flat_map(Mapping::keys)
                .filter_map(|name| name.as_str().map(str::to_string))
                .collect();
            let run = field(entry, "scripts")
                .and_then(Value::as_mapping)
                .and_then(|scripts| text(field(scripts, "run")));
            Some(DeclaredContainer {
                name,
                source,
                worker_ref,
                version: text(field(entry, "version")),
                start_after: string_list(field(entry, "start_after")),
                environment,
                run,
            })
        })
        .collect();

    Ok(ProjectDeclaration {
        file: file.into(),
        namespace: text(field(root, "namespace")),
        engine_url,
        engine_host,
        engine_port,
        startup_timeout: text(field(root, "startup_timeout")),
        stop_timeout: text(field(root, "stop_timeout")),
        containers,
    })
}

pub async fn read_project(file: &Path) -> Result<ProjectDeclaration, String> {
    let source = tokio::fs::read_to_string(file)
        .await
        .map_err(|error| error.to_string())?;
    parse_project(file.to_string_lossy(), &source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_worker_references() {
        assert_eq!(
            parse_worker_ref("path://../ade"),
            (ContainerSource::Path, "../ade".to_string())
        );
        assert_eq!(
            parse_worker_ref("package://api.workers.iii.dev/web"),
            (
                ContainerSource::Package,
                "api.workers.iii.dev/web".to_string()
            )
        );
        assert_eq!(
            parse_worker_ref("docker://redis"),
            (ContainerSource::Unknown, "docker://redis".to_string())
        );
    }

    #[test]
    fn parses_project_declarations_without_environment_values() {
        let project = parse_project(
            "/proj/worker-compose.yaml",
            r#"
namespace: demo
engine:
  url: ws://127.0.0.1:49134
startup_timeout: 30s
containers:
  console:
    worker: path://../ade
    start_after: [state]
    environment:
      SECRET: hidden
      PORT: 3113
    scripts:
      run: cargo run
  web:
    worker: package://api.workers.iii.dev/web
    version: 1.2.3
"#,
        )
        .unwrap();
        assert_eq!(project.namespace.as_deref(), Some("demo"));
        assert_eq!(project.engine_host.as_deref(), Some("127.0.0.1"));
        assert_eq!(project.engine_port, Some(49134));
        assert_eq!(project.containers.len(), 2);
        assert_eq!(project.containers[0].source, ContainerSource::Path);
        assert_eq!(project.containers[0].worker_ref, "../ade");
        assert_eq!(project.containers[0].start_after, ["state"]);
        assert_eq!(project.containers[0].environment, ["SECRET", "PORT"]);
        assert_eq!(project.containers[0].run.as_deref(), Some("cargo run"));
        assert_eq!(project.containers[1].version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn tolerates_missing_optional_sections() {
        let project = parse_project("compose.yaml", "containers:\n  empty: null\n").unwrap();
        assert_eq!(project.namespace, None);
        assert_eq!(project.engine_url, None);
        assert_eq!(project.containers[0].source, ContainerSource::Unknown);
        assert!(project.containers[0].environment.is_empty());
    }
}
