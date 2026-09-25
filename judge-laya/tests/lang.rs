//! The language/script detection port against outputs of laya 0.3.5's own
//! `lang.py` (`tests/fixtures/lang.json`, generated with the Python module).
use judge_laya::lang::{analyse, typed_decisions_workflow};
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn detection_matches_laya_python_fixtures() {
    let fixtures: BTreeMap<String, Value> =
        serde_json::from_str(include_str!("fixtures/lang.json")).unwrap();
    assert!(fixtures.len() >= 12);
    for (name, case) in fixtures {
        let expected = &case["expected"];
        let got = analyse(&case["input"]);
        assert_eq!(got.script, expected["script"], "{name}: script");
        assert_eq!(
            got.language.map(str::to_owned),
            expected["language"].as_str().map(str::to_owned),
            "{name}: language"
        );
        assert_eq!(got.is_english, expected["is_english"], "{name}: is_english");
        assert!(
            (got.diacritic_rate - expected["diacritic_rate"].as_f64().unwrap()).abs() < 1e-4,
            "{name}: diacritic_rate {} vs {}",
            got.diacritic_rate,
            expected["diacritic_rate"]
        );
    }
}

#[test]
fn workflow_signatures_need_the_exact_question_ids() {
    let full = ["urgency", "action", "category", "churn_risk", "needs_human"];
    assert_eq!(
        typed_decisions_workflow(full.into_iter()),
        Some("customer_service")
    );
    assert_eq!(typed_decisions_workflow(full[..4].iter().copied()), None);
    let extra = [
        "urgency",
        "action",
        "category",
        "churn_risk",
        "needs_human",
        "extra",
    ];
    assert_eq!(typed_decisions_workflow(extra.into_iter()), None);
}
