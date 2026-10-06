//! One container's declaration: read for the page's Settings tab and written
//! back as a container object through `compose::add`, which replaces only the
//! fields it is given. Secret-looking values never leave this worker: the page
//! sees them masked, and an edit re-sends the value already in the file.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as Json};
use serde_yaml::{Mapping, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct EnvVar {
    pub key: String,
    /// `None` when the value is a secret or empty.
    pub value: Option<String>,
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ContainerEntry {
    pub name: String,
    pub worker: String,
    pub version: Option<String>,
    pub start_after: Vec<String>,
    pub env_file: Vec<String>,
    pub environment: Vec<EnvVar>,
    pub run: Option<String>,
    /// The override as YAML text, as the page edits it.
    pub config_override: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct EnvPatch {
    #[serde(default)]
    pub set: BTreeMap<String, String>,
    #[serde(default)]
    pub unset: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct EditInput {
    pub container: String,
    /// Compose file on the daemon host; defaults to the daemon project.
    pub file: Option<String>,
    /// New worker source, e.g. `path:///home/me/other-checkout/web`. The
    /// container keeps its name, so the directory name must match it.
    pub worker: Option<String>,
    /// New `scripts.run`; an empty string removes it.
    pub run: Option<String>,
    pub start_after: Option<Vec<String>>,
    pub environment: Option<EnvPatch>,
    /// YAML text; empty removes the override.
    pub config_override: Option<String>,
    /// Caller-selected id for following the compose operation.
    pub operation_id: Option<String>,
}

fn is_template(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("${") && value.ends_with('}')
}

/// A value is masked when its key names a credential and it is a literal.
/// `${OPENAI_API_KEY}` only names where the value comes from, so it shows.
pub fn is_secret(key: &str, value: &str) -> bool {
    let key = key.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|word| key.contains(word))
        && !value.is_empty()
        && !is_template(value)
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(scalar)
        .collect()
}

fn entry_mapping(source: &str, container: &str) -> Result<Mapping, String> {
    let root: Value = serde_yaml::from_str(source).map_err(|error| error.to_string())?;
    root.get("containers")
        .and_then(|containers| containers.get(container))
        .and_then(Value::as_mapping)
        .cloned()
        .ok_or_else(|| format!("UNKNOWN_CONTAINER: {container} is not declared"))
}

pub fn read_entry(source: &str, container: &str) -> Result<ContainerEntry, String> {
    let entry = entry_mapping(source, container)?;
    let get = |key: &str| entry.get(Value::from(key));
    let environment = get("environment")
        .and_then(Value::as_mapping)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            let key = key.as_str()?.to_string();
            let value = scalar(value).unwrap_or_default();
            let secret = is_secret(&key, &value);
            Some(EnvVar {
                value: (!secret && !value.is_empty()).then_some(value),
                key,
                secret,
            })
        })
        .collect();
    let config_override = get("config_override")
        .filter(|value| !value.is_null())
        .map(|value| serde_yaml::to_string(value).unwrap_or_default());
    Ok(ContainerEntry {
        name: container.to_string(),
        worker: get("worker").and_then(scalar).unwrap_or_default(),
        version: get("version").and_then(scalar),
        start_after: strings(get("start_after")),
        env_file: strings(get("env_file")),
        environment,
        run: get("scripts").and_then(|s| s.get("run")).and_then(scalar),
        config_override,
    })
}

/// The container name the daemon derives from a worker reference: a path's
/// last directory, or a package's name.
pub fn derived_key(worker: &str) -> Option<String> {
    let spec = worker
        .trim()
        .trim_start_matches("path://")
        .trim_start_matches("package://");
    let last = spec.trim_end_matches('/').rsplit('/').next()?;
    let key = if worker.trim().starts_with("path://") || spec.starts_with(['/', '.', '~']) {
        last
    } else {
        last.split('@').next()?
    };
    (!key.is_empty() && key != "." && key != "..").then(|| key.to_string())
}

fn to_json(value: &Value) -> Result<Json, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

/// The container object for `compose::add`: `worker` plus every field the
/// edit changes, merged with what the file already declares.
pub fn container_object(source: &str, input: &EditInput) -> Result<Json, String> {
    let entry = entry_mapping(source, &input.container)?;
    let get = |key: &str| entry.get(Value::from(key));
    let current = get("worker")
        .and_then(scalar)
        .ok_or_else(|| format!("{} declares no worker", input.container))?;
    let worker = input.worker.clone().unwrap_or(current.clone());
    if derived_key(&worker).as_deref() != Some(input.container.as_str()) {
        return Err(format!(
            "KEY_MISMATCH: Compose names a worker after its directory or package, so {worker} \
             would be a new container instead of {}",
            input.container
        ));
    }

    let mut object = Map::new();
    object.insert("worker".into(), json!(worker));
    if let Some(run) = &input.run {
        let mut scripts = match get("scripts") {
            Some(Value::Mapping(scripts)) => to_json(&Value::Mapping(scripts.clone()))?,
            _ => json!({}),
        };
        let scripts_map = scripts.as_object_mut().expect("scripts is an object");
        if run.trim().is_empty() {
            scripts_map.remove("run");
        } else {
            scripts_map.insert("run".into(), json!(run.trim()));
        }
        object.insert("scripts".into(), scripts);
    }
    if let Some(start_after) = &input.start_after {
        object.insert("start_after".into(), json!(start_after));
    }
    if let Some(patch) = &input.environment {
        let mut environment = match get("environment") {
            Some(Value::Mapping(env)) => to_json(&Value::Mapping(env.clone()))?,
            _ => json!({}),
        };
        let environment_map = environment
            .as_object_mut()
            .expect("environment is an object");
        for key in &patch.unset {
            environment_map.remove(key);
        }
        for (key, value) in &patch.set {
            environment_map.insert(key.clone(), json!(value));
        }
        object.insert("environment".into(), environment);
    }
    if let Some(text) = &input.config_override {
        let value: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_yaml::from_str(text)
                .map_err(|error| format!("INVALID_CONFIG_OVERRIDE: {error}"))?
        };
        object.insert("config_override".into(), to_json(&value)?);
    }
    if object.len() == 1 && worker == current {
        return Err("NOTHING_TO_CHANGE: the edit names no field".to_string());
    }
    Ok(Json::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"
namespace: demo
containers:
  harness-e2e:
    worker: path:///home/me/wt/harness-e2e
    env_file: [.env]
    config_override:
      data_dir: ~/.iii/data/harness-e2e
    environment:
      RUST_LOG: info
      OPENAI_API_KEY: sk-literal
      ZAI_API_KEY: ${ZAI_API_KEY}
      PORT: 3113
    start_after: [state]
    scripts:
      pre_run: cargo build
      run: ./target/debug/harness-e2e
  database:
    worker: package://api.workers.iii.dev/database
    version: "0.5.17"
"#;

    #[test]
    fn masks_literal_secrets_but_shows_templates() {
        let entry = read_entry(FILE, "harness-e2e").unwrap();
        let env: BTreeMap<_, _> = entry
            .environment
            .iter()
            .map(|v| (v.key.as_str(), (v.value.as_deref(), v.secret)))
            .collect();
        assert_eq!(env["RUST_LOG"], (Some("info"), false));
        assert_eq!(env["OPENAI_API_KEY"], (None, true));
        assert_eq!(env["ZAI_API_KEY"], (Some("${ZAI_API_KEY}"), false));
        assert_eq!(env["PORT"], (Some("3113"), false));
        assert_eq!(entry.run.as_deref(), Some("./target/debug/harness-e2e"));
        assert_eq!(entry.env_file, [".env"]);
        assert!(entry.config_override.unwrap().contains("data_dir"));
    }

    #[test]
    fn an_edit_keeps_secret_values_and_other_scripts() {
        let object = container_object(
            FILE,
            &EditInput {
                container: "harness-e2e".into(),
                run: Some("cargo run --locked --bin harness-e2e".into()),
                environment: Some(EnvPatch {
                    set: [("RUST_LOG".to_string(), "debug".to_string())].into(),
                    unset: vec!["PORT".into()],
                }),
                ..EditInput::default()
            },
        )
        .unwrap();
        assert_eq!(object["worker"], "path:///home/me/wt/harness-e2e");
        assert_eq!(object["scripts"]["pre_run"], "cargo build");
        assert_eq!(
            object["scripts"]["run"],
            "cargo run --locked --bin harness-e2e"
        );
        assert_eq!(object["environment"]["OPENAI_API_KEY"], "sk-literal");
        assert_eq!(object["environment"]["RUST_LOG"], "debug");
        assert!(object["environment"].get("PORT").is_none());
        assert!(
            object.get("start_after").is_none(),
            "untouched fields stay out"
        );
    }

    #[test]
    fn a_new_path_must_keep_the_container_name() {
        let ok = EditInput {
            container: "harness-e2e".into(),
            worker: Some("path:///home/me/main/harness-e2e".into()),
            ..EditInput::default()
        };
        assert_eq!(
            container_object(FILE, &ok).unwrap()["worker"],
            "path:///home/me/main/harness-e2e"
        );
        let renamed = EditInput {
            worker: Some("path:///home/me/harness-e2e-benchmarks".into()),
            ..ok
        };
        assert!(container_object(FILE, &renamed)
            .unwrap_err()
            .starts_with("KEY_MISMATCH"));
    }

    #[test]
    fn refuses_an_empty_edit_and_unknown_containers() {
        let empty = EditInput {
            container: "database".into(),
            ..EditInput::default()
        };
        assert!(container_object(FILE, &empty)
            .unwrap_err()
            .starts_with("NOTHING_TO_CHANGE"));
        assert!(read_entry(FILE, "nope")
            .unwrap_err()
            .starts_with("UNKNOWN_CONTAINER"));
    }

    #[test]
    fn derives_keys_like_the_daemon() {
        assert_eq!(derived_key("path:///a/b/web/").as_deref(), Some("web"));
        assert_eq!(
            derived_key("package://api.workers.iii.dev/web").as_deref(),
            Some("web")
        );
        assert_eq!(derived_key("web@1.2.0").as_deref(), Some("web"));
        assert_eq!(derived_key("path://.."), None);
    }
}
