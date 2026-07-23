//! The checked-in integration fixtures.

mod console_streamed_text;
mod dsl;
mod exactly_once_function;
mod multi_turn_traces;
mod reseed_parked_message;
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
        console_streamed_text::scenario(),
        exactly_once_function::scenario(),
        multi_turn_traces::scenario(),
        reseed_parked_message::scenario(),
        streamed_text::scenario(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_is_unique_and_valid() {
        let fixtures = all();
        assert_eq!(fixtures.len(), 5);
        let mut slugs = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for fixture in fixtures {
            fixture.validate().unwrap();
            assert!(slugs.insert(fixture.slug));
            assert!(ids.insert(fixture.scenario.id));
        }
    }
}
