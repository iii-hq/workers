//! Execute the provider's actual wire serializer without building its UI.
//! Only ABI-compatible data types are adapted here; encoding/serialization
//! implementations below are imported unchanged from their owning workers.
extern crate self as llm_router;

pub mod types {
    pub mod model {
        #[derive(Clone)]
        pub struct AgentFunction {
            pub name:String,
            pub description:String,
            pub parameters:serde_json::Value,
            pub label:Option<String>,
            pub execution_mode:Option<String>,
        }
    }
}
#[path = "../../llm-router/src/provider_scaffold/names.rs"]
pub mod real_names;
#[path = "../../provider-openai-codex/src/wire/tools.rs"]
pub mod real_provider_tools;
pub mod provider_scaffold {
    pub use crate::real_names as names;
}
pub mod wire {
    pub mod names {
        pub use crate::provider_scaffold::names::{decode_tool_name,encode_tool_name};
    }
    pub use crate::real_provider_tools as tools;
}

#[test]
fn joint_policy_tools_round_trip_through_real_provider_without_native_shell() {
    let offered:Vec<_>=sentinel::functions::JOINT_ALLOW.iter().map(|id|types::model::AgentFunction {
        name:(*id).into(),description:"Sentinel bounded operation".into(),parameters:serde_json::json!({"type":"object"}),label:None,execution_mode:Some("sequential".into())
    }).collect();
    let wire=wire::tools::functions_to_wire(&offered);
    let names:Vec<_>=wire.iter().map(|tool|{
        assert_eq!(tool["type"],"function");
        provider_scaffold::names::decode_tool_name(tool["name"].as_str().unwrap())
    }).collect();
    assert_eq!(names,vec!["sentinel::diagnosis::record","sentinel::source::read"]);
    assert!(wire.iter().all(|tool|tool["type"]!="local_shell" && tool["type"]!="computer"));
    assert!(wire::tools::functions_to_wire(&[]).is_empty());
}
