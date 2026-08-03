//! The checked-in integration fixtures.

mod console_streamed_text;
mod database_row_wake;
mod direct_spawn_leaf_pipeline;
mod dsl;
mod engine_restart_recovery;
mod exactly_once_function;
mod leaf_denied_control_plane;
mod multi_turn_traces;
mod queued_message_edit_unqueue;
mod reseed_parked_message;
mod spawn_reuse_guard;
mod standing_wake_delivery;
mod state_worker_sidecar;
mod stop_cancel_cascade;
mod streamed_text;
mod timer_wake;
mod wake_expiry_notice;

use crate::evidence_data::RunEvidence;
use crate::fixtures::ScenarioFixture;

pub type VerifyFn = fn(&RunEvidence) -> anyhow::Result<()>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioDriver {
    Direct,
    Playground,
}

/// Every fixture, in stable slug order.
pub fn all() -> Vec<ScenarioFixture> {
    vec![
        console_streamed_text::scenario(),
        database_row_wake::scenario(),
        direct_spawn_leaf_pipeline::scenario(),
        engine_restart_recovery::scenario(),
        exactly_once_function::scenario(),
        leaf_denied_control_plane::scenario(),
        multi_turn_traces::scenario(),
        standing_wake_delivery::scenario(),
        state_worker_sidecar::scenario(),
        reseed_parked_message::scenario(),
        spawn_reuse_guard::scenario(),
        stop_cancel_cascade::scenario(),
        queued_message_edit_unqueue::scenario(),
        streamed_text::scenario(),
        wake_expiry_notice::scenario(),
        timer_wake::scenario(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_is_unique_and_valid() {
        let fixtures = all();
        assert_eq!(fixtures.len(), 16);
        let mut slugs = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for fixture in fixtures {
            fixture.validate().unwrap();
            assert!(slugs.insert(fixture.slug));
            assert!(ids.insert(fixture.scenario.id));
        }
    }
}
