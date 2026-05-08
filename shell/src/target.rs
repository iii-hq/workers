//! Per-call target selector shared by `shell::fs::*` and `shell::exec*`
//! request bodies. Defaults to host when omitted on the wire.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    #[default]
    Host,
    Sandbox {
        sandbox_id: Uuid,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_to_host_when_absent() {
        #[derive(Deserialize)]
        struct Wrap {
            #[serde(default)]
            target: Target,
        }
        let w: Wrap = serde_json::from_value(json!({})).unwrap();
        assert_eq!(w.target, Target::Host);
    }

    #[test]
    fn host_round_trips() {
        let v = serde_json::to_value(Target::Host).unwrap();
        assert_eq!(v, json!({ "kind": "host" }));
        let back: Target = serde_json::from_value(v).unwrap();
        assert_eq!(back, Target::Host);
    }

    #[test]
    fn sandbox_carries_uuid() {
        let id = Uuid::new_v4();
        let t = Target::Sandbox { sandbox_id: id };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "sandbox");
        assert_eq!(v["sandbox_id"], id.to_string());
        let back: Target = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }
}
