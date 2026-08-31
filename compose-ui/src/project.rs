use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use tokio::process::Command;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListeningPort {
    pub port: u16,
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProjectContainer {
    pub name: String,
    pub source: ContainerSource,
    #[serde(rename = "ref")]
    pub worker_ref: String,
    pub version: Option<String>,
    pub start_after: Vec<String>,
    pub environment: Vec<String>,
    pub run: Option<String>,
    pub pid: Option<u32>,
    pub ports: Vec<ListeningPort>,
}

impl ProjectContainer {
    pub fn from_declared(
        declared: DeclaredContainer,
        pid: Option<u32>,
        ports: Vec<ListeningPort>,
    ) -> Self {
        Self {
            name: declared.name,
            source: declared.source,
            worker_ref: declared.worker_ref,
            version: declared.version,
            start_after: declared.start_after,
            environment: declared.environment,
            run: declared.run,
            pid,
            ports,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProjectResult {
    pub file: String,
    pub namespace: Option<String>,
    pub engine_url: Option<String>,
    pub engine_host: Option<String>,
    pub engine_port: Option<u16>,
    pub startup_timeout: Option<String>,
    pub stop_timeout: Option<String>,
    pub daemon_pid: Option<u32>,
    pub daemon_ports: Vec<ListeningPort>,
    pub containers: Vec<ProjectContainer>,
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

pub fn parse_lsof(output: &str) -> HashMap<u32, Vec<ListeningPort>> {
    let mut by_pid: HashMap<u32, Vec<ListeningPort>> = HashMap::new();
    let mut pid = None;
    for line in output.lines() {
        if let Some(raw_pid) = line.strip_prefix('p') {
            pid = raw_pid.parse().ok();
            if let Some(pid) = pid {
                by_pid.entry(pid).or_default();
            }
            continue;
        }
        let (Some(pid), Some(name)) = (pid, line.strip_prefix('n')) else {
            continue;
        };
        let Some((address, raw_port)) = name.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = raw_port.parse::<u16>() else {
            continue;
        };
        let ports = by_pid.entry(pid).or_default();
        if !ports
            .iter()
            .any(|entry| entry.port == port && entry.address == address)
        {
            ports.push(ListeningPort {
                port,
                address: address.to_string(),
            });
        }
    }
    for ports in by_pid.values_mut() {
        ports.sort_by_key(|entry| entry.port);
    }
    by_pid
}

pub async fn listening_ports(pids: &[u32]) -> HashMap<u32, Vec<ListeningPort>> {
    let unique: HashSet<u32> = pids.iter().copied().filter(|pid| *pid > 0).collect();
    if unique.is_empty() {
        return HashMap::new();
    }
    let mut unique: Vec<u32> = unique.into_iter().collect();
    unique.sort_unstable();
    let pids = unique
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("lsof")
            .args(["-nP", "-a", "-p", &pids, "-iTCP", "-sTCP:LISTEN", "-Fpn"])
            .output(),
    )
    .await;
    match output {
        Ok(Ok(output)) => parse_lsof(&String::from_utf8_lossy(&output.stdout)),
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "could not inspect listening ports with lsof");
            HashMap::new()
        }
        Err(_) => {
            tracing::warn!("lsof timed out while inspecting compose processes");
            HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_worker_references() {
        assert_eq!(
            parse_worker_ref("path://../console"),
            (ContainerSource::Path, "../console".to_string())
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
    worker: path://../console
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
        assert_eq!(project.containers[0].worker_ref, "../console");
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

    #[test]
    fn parses_and_deduplicates_lsof_records() {
        let ports = parse_lsof("p42\nn*:3113\nn*:3113\nn127.0.0.1:49134\np7\nn[::1]:8080\n");
        assert_eq!(
            ports[&42],
            [
                ListeningPort {
                    port: 3113,
                    address: "*".to_string()
                },
                ListeningPort {
                    port: 49134,
                    address: "127.0.0.1".to_string()
                }
            ]
        );
        assert_eq!(ports[&7][0].address, "[::1]");
    }

    #[test]
    fn ignores_malformed_lsof_lines() {
        let ports = parse_lsof("n*:9999\npnope\nn*:abc\np5\nCOMMAND\nn*:80\n");
        assert_eq!(ports[&5][0].port, 80);
        assert_eq!(ports.len(), 1);
    }
}
