//! Authored scenario modules and their registry.
//!
//! One Rust module per scenario: each `src/scenarios/<slug>.rs` builds the
//! authored scenario data through the typed builders in [`builder`], defines
//! a `verify` function over the returned [`RunEvidence`] dataset, and
//! registers both in [`all`]. There is no YAML layer — the authored shape is
//! the runner's own data model, enforced at `cargo build`, and it is never
//! serialized. Compilation and serde round trips are enforced by tests
//! without checking in a second copy of each scenario.

pub mod builder;

mod console_streamed_text;
mod crash_recovery_507;
mod exactly_once_function;
mod hold_mutation_505;
mod hook_held_release_506;
mod streamed_text;

use crate::evidence_data::RunEvidence;
use crate::types::scenario::AuthoredScenarioV1;

/// Scenario-specific checks written in plain Rust over the collected run
/// dataset. The runner enforces the floor (completed turn, script fully
/// consumed, clean send) before calling this, and catches panics, so
/// `assert!`/`assert_eq!` are allowed; prefer `anyhow::ensure!` where a
/// message helps.
pub type VerifyFn = fn(&RunEvidence) -> anyhow::Result<()>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioDriver {
    Direct,
    Console,
}

/// One authored scenario, its verify function, and the stable slug used by
/// `--scenario` selection.
#[derive(Debug, Clone)]
pub struct RegisteredScenario {
    pub slug: String,
    pub authored: AuthoredScenarioV1,
    pub driver: ScenarioDriver,
    pub verify: VerifyFn,
}

/// Every authored scenario, in stable slug order.
pub fn all() -> Vec<RegisteredScenario> {
    vec![
        register("crash-recovery-507", crash_recovery_507::scenario()),
        register_console("console-streamed-text", console_streamed_text::scenario()),
        register("exactly-once-function", exactly_once_function::scenario()),
        register("hold-mutation-505", hold_mutation_505::scenario()),
        register("hook-held-release-506", hook_held_release_506::scenario()),
        register("streamed-text", streamed_text::scenario()),
    ]
}

fn register(slug: &str, scenario: builder::Scenario) -> RegisteredScenario {
    RegisteredScenario {
        slug: slug.to_string(),
        authored: scenario.authored,
        driver: ScenarioDriver::Direct,
        verify: scenario.verify,
    }
}

fn register_console(slug: &str, scenario: builder::Scenario) -> RegisteredScenario {
    RegisteredScenario {
        slug: slug.to_string(),
        authored: scenario.authored,
        driver: ScenarioDriver::Console,
        verify: scenario.verify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expand::compile_scenario;
    use crate::types::scenario::validate_scenario_id;

    /// Slugs are stable `--scenario` selectors and filesystem-safe labels.
    fn validate_slug(slug: &str) {
        assert!(
            !slug.is_empty()
                && slug
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "slug {slug:?} must contain only ASCII letters, digits, '-' or '_'"
        );
    }

    #[test]
    fn every_scenario_registers_exactly_once() {
        let registered = all();
        assert!(!registered.is_empty());
        let mut slugs = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for entry in &registered {
            validate_slug(&entry.slug);
            validate_scenario_id(&entry.authored.id).unwrap();
            assert!(
                slugs.insert(entry.slug.clone()),
                "slug {:?} registered more than once",
                entry.slug
            );
            assert!(
                ids.insert(entry.authored.id.clone()),
                "scenario id {:?} registered more than once",
                entry.authored.id
            );
        }
    }

    #[test]
    fn every_scenario_compiles() {
        for entry in all() {
            assert!(
                !entry.authored.send.message.is_empty(),
                "{}: authored send message must not be empty",
                entry.slug
            );
            compile_scenario(&entry.authored, "base prompt\n")
                .unwrap_or_else(|error| panic!("{} does not compile: {error:#}", entry.slug));
        }
    }
}
