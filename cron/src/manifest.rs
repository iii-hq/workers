//! Manifest generation for the cron worker.

use serde::Serialize;
use serde_json::Value;

use crate::on_tick::{CronArgs, CronTick, ON_TICK_DESCRIPTION, ON_TICK_FN};

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: &'static str,
    pub functions: Vec<FunctionManifest>,
}

#[derive(Serialize)]
pub struct FunctionManifest {
    pub id: &'static str,
    pub description: &'static str,
    pub request_format: Value,
    /// What observers observe: the handler's return value.
    pub response_format: Value,
    /// Observers' args.
    pub args_format: Value,
    /// Only this worker calls it; anyone may observe it.
    pub private: bool,
}

pub fn build_manifest() -> ModuleManifest {
    let tick = serde_json::to_value(schemars::schema_for!(CronTick)).unwrap_or(Value::Null);
    ModuleManifest {
        name: "cron",
        functions: vec![FunctionManifest {
            id: ON_TICK_FN,
            description: ON_TICK_DESCRIPTION,
            request_format: tick.clone(),
            response_format: tick,
            args_format: serde_json::to_value(schemars::schema_for!(CronArgs))
                .unwrap_or(Value::Null),
            private: true,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lists_on_tick_with_its_formats() {
        let v = serde_json::to_value(build_manifest()).unwrap();
        assert_eq!(v["name"], "cron");
        assert_eq!(v["functions"].as_array().unwrap().len(), 1);
        let on_tick = &v["functions"][0];
        assert_eq!(on_tick["id"], "cron::on-tick");
        assert_eq!(on_tick["private"], true);
        assert_eq!(
            on_tick["args_format"]["required"],
            serde_json::json!(["expression"])
        );
        assert_eq!(on_tick["request_format"], on_tick["response_format"]);
        assert!(on_tick["response_format"]["properties"]["scheduled_time"].is_object());
    }
}
