//! Manifest generation for the cron worker.

use serde::Serialize;
use serde_json::Value;

use crate::config::CronConfig;
use crate::on_tick::{CronArgs, CronTick, ON_TICK_DESCRIPTION, ON_TICK_FN};
use crate::trigger::CronTriggerSpec;
use crate::TRIGGER_TYPE;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: &'static str,
    pub trigger_types: Vec<TriggerTypeManifest>,
    pub functions: Vec<FunctionManifest>,
    pub config_schema: Value,
}

#[derive(Serialize)]
pub struct FunctionManifest {
    pub id: &'static str,
    pub description: &'static str,
    pub request_format: Value,
    /// Observers' args (owner-delivered function).
    pub args_format: Value,
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
        functions: vec![FunctionManifest {
            id: ON_TICK_FN,
            description: ON_TICK_DESCRIPTION,
            request_format: serde_json::to_value(schemars::schema_for!(CronTick))
                .unwrap_or(Value::Null),
            args_format: serde_json::to_value(schemars::schema_for!(CronArgs))
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
        assert_eq!(v["functions"][0]["id"], "cron::on-tick");
        assert_eq!(
            v["functions"][0]["args_format"]["required"],
            serde_json::json!(["expression"])
        );
        assert!(v["config_schema"].is_object());
    }
}
