use serde::Serialize;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

fn default_config() -> serde_json::Value {
    let raw = include_str!("../iii.worker.yaml");
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(raw)
        .ok()
        .and_then(|root| root.get("config").cloned())
        .and_then(|node| serde_yaml::from_value::<crate::config::WorkerConfig>(node).ok())
        .and_then(|cfg| serde_json::to_value(cfg).ok());

    parsed.unwrap_or_else(|| {
        serde_json::to_value(crate::config::WorkerConfig::default())
            .unwrap_or_else(|_| serde_json::json!({}))
    })
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Trigger-model approval gate. Intercepts function calls listed in approval_required, returns a pending_approval tool result immediately, and executes the underlying function on approval::resolve. Resolutions stitch into the agent's next turn via approval::list_undelivered + approval::ack_delivered."
            .to_string(),
        default_config: default_config(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m = build_manifest();
        let json = serde_json::to_string_pretty(&m).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["description"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn default_config_matches_shipped_worker_yaml() {
        let raw = include_str!("../iii.worker.yaml");
        let root: serde_yaml::Value = serde_yaml::from_str(raw).unwrap();
        let expected: crate::config::WorkerConfig =
            serde_yaml::from_value(root.get("config").unwrap().clone()).unwrap();

        let actual: crate::config::WorkerConfig =
            serde_json::from_value(build_manifest().default_config).unwrap();

        assert_eq!(actual, expected);
        assert!(
            actual.rules.iter().any(|r| {
                r.permission == "approval::*"
                    && r.pattern == "*"
                    && r.action == crate::rules::Action::Allow
            }),
            "manifest defaults must include the self-allow rule from iii.worker.yaml",
        );
        assert_eq!(
            actual.rules.last().map(|r| r.action),
            Some(crate::rules::Action::Ask),
            "manifest defaults must preserve the catch-all ask rule",
        );
    }
}
