//! Worker manifest. Emitted when `context-compaction --manifest` is invoked
//! so the harness manifest generator and registry can read worker metadata
//! without booting an iii engine.

use serde::Serialize;

pub const SKILL_ID: &str = "context-compaction";
pub const SKILL_BODY: &str = include_str!("../skill.md");

#[derive(Debug, Serialize)]
pub struct Manifest {
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub functions: Vec<&'static str>,
    pub subscriptions: Vec<&'static str>,
    pub skill_id: &'static str,
    pub skill: &'static str,
}

pub fn build() -> Manifest {
    Manifest {
        name: "context-compaction",
        version: env!("CARGO_PKG_VERSION"),
        description: "Out-of-band session-history compactor (subscribes to agent::events::TurnEnd).",
        functions: vec![],
        subscriptions: vec!["agent::events"],
        skill_id: SKILL_ID,
        skill: SKILL_BODY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_carries_no_llm_facing_functions() {
        let m = build();
        assert!(m.functions.is_empty(), "compactor is not LLM-facing");
        assert_eq!(m.subscriptions, vec!["agent::events"]);
    }

    #[test]
    fn manifest_includes_skill_body() {
        let m = build();
        assert!(m.skill.contains("compaction"));
    }
}
