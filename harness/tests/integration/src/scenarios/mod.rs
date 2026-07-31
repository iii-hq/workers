//! The checked-in integration fixtures.

mod coalesced_fire;
mod console_queued_message_streaming;
mod console_streamed_text;
mod dsl;
mod engine_restart_recovery;
mod exactly_once_function;
mod join_spec_mismatch;
mod late_join_replay;
mod multi_turn_traces;
mod queued_message_edit_unqueue;
mod reaction_policy_inheritance;
mod reaction_unregisters_run;
mod reseed_parked_message;
mod state_worker_sidecar;
mod stop_cancel_cascade;
mod streamed_text;

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
        coalesced_fire::scenario(),
        console_queued_message_streaming::scenario(),
        console_streamed_text::scenario(),
        engine_restart_recovery::scenario(),
        exactly_once_function::scenario(),
        join_spec_mismatch::scenario(),
        late_join_replay::scenario(),
        multi_turn_traces::scenario(),
        reaction_policy_inheritance::scenario(),
        reaction_unregisters_run::scenario(),
        state_worker_sidecar::scenario(),
        reseed_parked_message::scenario(),
        stop_cancel_cascade::scenario(),
        queued_message_edit_unqueue::scenario(),
        streamed_text::scenario(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_is_unique_and_valid() {
        let fixtures = all();
        assert_eq!(fixtures.len(), 15);
        let mut slugs = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for fixture in fixtures {
            fixture.validate().unwrap();
            assert!(slugs.insert(fixture.slug));
            assert!(ids.insert(fixture.scenario.id));
        }
    }
}
