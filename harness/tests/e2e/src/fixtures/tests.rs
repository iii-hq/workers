use super::*;

#[test]
fn all_selection_returns_the_checked_in_fixtures() {
    let fixtures = scenario_fixtures("all").unwrap();
    let ids = fixtures
        .iter()
        .map(|fixture| fixture.scenario.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        ids,
        std::collections::BTreeSet::from([
            "E2E-001", "E2E-002", "E2E-003", "E2E-004", "E2E-005", "E2E-006", "E2E-007", "E2E-008",
            "E2E-009", "E2E-010", "UI-001", "UI-002"
        ])
    );
    assert_eq!(
        fixtures
            .iter()
            .filter(|fixture| fixture.driver == crate::scenarios::ScenarioDriver::Direct)
            .count(),
        10
    );
}

#[test]
fn explicit_selection_accepts_slug_or_id() {
    assert_eq!(
        scenario_fixtures("streamed-text").unwrap()[0].scenario.id,
        "E2E-001"
    );
    assert_eq!(
        scenario_fixtures("E2E-002").unwrap()[0].slug,
        "exactly-once-function"
    );
    assert!(scenario_fixtures("missing").is_err());
}

#[test]
fn duplicate_identities_are_rejected() {
    let mut fixtures = crate::scenarios::all();
    fixtures[1].scenario.id = fixtures[0].scenario.id.clone();
    let error = super::discovery::select_fixtures(fixtures, "all").unwrap_err();
    assert!(format!("{error:#}").contains("duplicate scenario id"));

    let mut fixtures = crate::scenarios::all();
    fixtures[1].slug = fixtures[0].slug.clone();
    let error = super::discovery::select_fixtures(fixtures, "all").unwrap_err();
    assert!(format!("{error:#}").contains("duplicate scenario slug"));
}

#[test]
fn malformed_terminal_sequence_is_rejected() {
    let mut fixture = crate::scenarios::all().remove(0);
    fixture.script.generations[0].frames.pop();
    let error = fixture.validate().unwrap_err();
    assert!(format!("{error:#}").contains("terminal frame"));
}
