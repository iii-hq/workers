//! Manifest generation for the cron worker.

use serde::Serialize;
use serde_json::Value;

use crate::config::CronConfig;
use crate::trigger::CronTriggerSpec;
use crate::TRIGGER_TYPE;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: &'static str,
    pub trigger_types: Vec<TriggerTypeManifest>,
    pub config_schema: Value,
}

#[derive(Serialize)]
pub struct TriggerTypeManifest {
    pub id: &'static str,
    pub description: &'static str,
    pub trigger_request_format: Value,
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: "cron",
        trigger_types: vec![TriggerTypeManifest {
            id: TRIGGER_TYPE,
            description: "Cron-based scheduled triggers",
            trigger_request_format: serde_json::to_value(schemars::schema_for!(CronTriggerSpec))
                .unwrap_or(Value::Null),
        }],
        config_schema: CronConfig::json_schema(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_cron_trigger_type_and_config_schema() {
        let v = serde_json::to_value(build_manifest()).unwrap();
        assert_eq!(v["name"], "cron");
        assert_eq!(v["trigger_types"][0]["id"], "cron");
        assert!(v["config_schema"].is_object());
    }
}
