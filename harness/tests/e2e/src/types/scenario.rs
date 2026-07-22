//! Strict scenario runtime and result contracts.

mod compiled;
mod result;

pub use compiled::*;
pub use result::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_defaults_are_safe() {
        let timeouts = DeadlinesV1::default();
        assert_eq!(timeouts.readiness_ms, 60_000);
        assert_eq!(timeouts.scenario_ms, 60_000);
        assert_eq!(timeouts.teardown_ms, 15_000);
    }
}
