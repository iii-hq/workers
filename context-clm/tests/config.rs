//! Defaults, parsing and validation of the `context-clm` configuration entry.
use context_clm::config::{ClmConfig, EditGate, PersistentNudge, DEFAULT_DATA_DIR};
use serde_json::{json, Value};

#[test]
fn defaults_match_the_spec() {
    assert_eq!(
        ClmConfig::default().to_json(),
        json!({
            "budget_tokens": 65536,
            "reserve_tokens": 2048,
            "nudge_ratios": [0.25, 0.5, 0.75],
            "persistent_nudge": "adaptive",
            "max_rollbacks": 50,
            "rollback_margin_tokens": 2048,
            "edit_gate": "fit",
            "observation_max_chars": 10000,
            "strip_thinking": true,
            "step_log": true,
            "data_dir": "data/context-clm"
        })
    );
    assert_eq!(
        ClmConfig::from_json(&json!({})).unwrap(),
        ClmConfig::default()
    );
}

#[test]
fn persistent_nudge_accepts_adaptive_off_and_ratios() {
    let parse = |v: Value| ClmConfig::from_json(&json!({ "persistent_nudge": v }));
    assert_eq!(
        parse(json!("adaptive")).unwrap().persistent_nudge,
        PersistentNudge::Adaptive
    );
    assert_eq!(
        parse(json!("off")).unwrap().persistent_nudge,
        PersistentNudge::Off
    );
    assert_eq!(
        parse(json!(0.9)).unwrap().persistent_nudge,
        PersistentNudge::Ratio(0.9)
    );
    assert_eq!(
        parse(json!(1)).unwrap().persistent_nudge,
        PersistentNudge::Ratio(1.0)
    );
    assert!(parse(json!(0)).is_err());
    assert!(parse(json!(1.5)).is_err());
    assert!(parse(json!("sometimes")).is_err());
    let ratio = ClmConfig {
        persistent_nudge: PersistentNudge::Ratio(0.8),
        ..ClmConfig::default()
    };
    assert_eq!(ratio.to_json()["persistent_nudge"], json!(0.8));
}

#[test]
fn edit_gate_is_lowercase() {
    let config = ClmConfig::from_json(&json!({"edit_gate": "shrink"})).unwrap();
    assert_eq!(config.edit_gate, EditGate::Shrink);
    assert!(ClmConfig::from_json(&json!({"edit_gate": "Fit"})).is_err());
}

#[test]
fn invalid_values_are_rejected() {
    for bad in [
        json!({"budget_tokens": 2048}),
        json!({"budget_tokens": 1000, "reserve_tokens": 1000}),
        json!({"nudge_ratios": [0.5, 1.0]}),
        json!({"nudge_ratios": [0.0]}),
        json!({"observation_max_chars": 0}),
        // a rollback must land above zero: budget - reserve > margin
        json!({"rollback_margin_tokens": 63488}),
        json!({"budget_tokens": 4096, "reserve_tokens": 2048}),
        json!({"budget_tokens": "big"}),
        json!({"unknown_knob": true}),
    ] {
        assert!(ClmConfig::from_json(&bad).is_err(), "{bad}");
    }
    assert!(ClmConfig::from_json(&json!({"rollback_margin_tokens": 63487})).is_ok());
}

#[test]
fn blank_data_dir_falls_back() {
    let config = ClmConfig::from_json(&json!({"data_dir": "  "})).unwrap();
    assert_eq!(config.data_dir, DEFAULT_DATA_DIR);
}

#[test]
fn schema_describes_every_field_and_carries_the_defaults() {
    let schema = ClmConfig::json_schema();
    for field in ClmConfig::default().to_json().as_object().unwrap().keys() {
        assert!(schema["properties"][field].is_object(), "{field}");
    }
    assert_eq!(schema["example"], ClmConfig::default().to_json());
}

#[test]
fn schema_carries_the_single_field_bounds_validate_enforces() {
    // So the configuration worker refuses at set time what boot would refuse.
    let schema = ClmConfig::json_schema();
    let items = &schema["properties"]["nudge_ratios"]["items"];
    assert_eq!(
        (&items["exclusiveMinimum"], &items["exclusiveMaximum"]),
        (&json!(0), &json!(1))
    );
    assert_eq!(
        schema["properties"]["observation_max_chars"]["minimum"],
        json!(1.0)
    );
    let ratio: Vec<&Value> = schema["definitions"]["PersistentNudge"]["anyOf"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["type"] == "number")
        .collect();
    assert_eq!(ratio.len(), 1);
    assert_eq!(
        (&ratio[0]["exclusiveMinimum"], &ratio[0]["maximum"]),
        (&json!(0), &json!(1))
    );
}
