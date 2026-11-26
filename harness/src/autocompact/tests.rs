use serde_json::{Map, Value, json};

use super::*;
use crate::compaction::Status;
use crate::config::{AutocompactConfig, AutocompactMode};
use crate::projects::Project;

fn project(id: &str, name: &str, autocompact: Value) -> Project {
    Project {
        id: id.into(),
        name: name.into(),
        color: "blue".into(),
        folders: vec![format!("/work/{name}")],
        account: crate::projects::default_account(),
        autocompact,
        created_at_ms: 1,
        updated_at_ms: 1,
        extra: Map::new(),
    }
}

fn metadata(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn global(tokens: Option<u64>) -> AutocompactConfig {
    AutocompactConfig {
        mode: if tokens.is_some() {
            AutocompactMode::Tokens
        } else {
            AutocompactMode::Off
        },
        tokens,
        instructions: None,
    }
}

fn acme() -> Project {
    project(
        "prj_acme",
        "acme-web",
        json!({ "mode": "tokens", "tokens": 160_000 }),
    )
}

fn mobile() -> Project {
    project("prj_mobile", "mobile", json!({ "mode": "off" }))
}

fn plain() -> Project {
    project("prj_plain", "plain", json!({ "mode": "inherit" }))
}

fn acme_rule() -> Rule {
    Rule {
        scope: RuleScope::Project,
        project_id: Some("prj_acme".into()),
        project_name: Some("acme-web".into()),
    }
}

/// Spec §3.2.1: the first level that is not `inherit` decides and names the
/// rule; the default is off, decided by the global.
#[test]
fn the_chain_resolves_conversation_then_project_then_global() {
    let projects = [acme(), mobile(), plain()];
    let cases = [
        // (conversation metadata, global, setting, rule)
        (json!({}), global(None), None, Rule::global()),
        (
            json!({}),
            global(Some(400_000)),
            Some(400_000),
            Rule::global(),
        ),
        (
            json!({ "project_id": "prj_acme" }),
            global(Some(400_000)),
            Some(160_000),
            acme_rule(),
        ),
        (
            json!({ "project_id": "prj_mobile" }),
            global(Some(400_000)),
            None,
            Rule {
                scope: RuleScope::Project,
                project_id: Some("prj_mobile".into()),
                project_name: Some("mobile".into()),
            },
        ),
        // A project that inherits leaves it to the global.
        (
            json!({ "project_id": "prj_plain" }),
            global(Some(400_000)),
            Some(400_000),
            Rule::global(),
        ),
        // The conversation's own value wins over its project's.
        (
            json!({ "project_id": "prj_acme", "autocompact": { "mode": "tokens", "tokens": 600_000 } }),
            global(Some(400_000)),
            Some(600_000),
            Rule::session(),
        ),
        (
            json!({ "project_id": "prj_acme", "autocompact": { "mode": "off" } }),
            global(Some(400_000)),
            None,
            Rule::session(),
        ),
        (
            json!({ "project_id": "prj_acme", "autocompact": { "mode": "inherit" } }),
            global(None),
            Some(160_000),
            acme_rule(),
        ),
        // A malformed conversation value reads as inherit.
        (
            json!({ "project_id": "prj_acme", "autocompact": { "mode": "tokens" } }),
            global(None),
            Some(160_000),
            acme_rule(),
        ),
        (
            json!({ "project_id": "prj_acme", "autocompact": { "mode": "tokens", "tokens": 5 } }),
            global(None),
            Some(160_000),
            acme_rule(),
        ),
        (
            json!({ "project_id": "prj_acme", "autocompact": "160k" }),
            global(None),
            Some(160_000),
            acme_rule(),
        ),
        // No project, a project that is gone, `none`: the global.
        (
            json!({ "project_id": "none" }),
            global(Some(400_000)),
            Some(400_000),
            Rule::global(),
        ),
        (
            json!({ "project_id": "prj_gone" }),
            global(Some(400_000)),
            Some(400_000),
            Rule::global(),
        ),
    ];
    for (conversation, global, setting, rule) in cases {
        let resolved = resolve_with(&metadata(conversation.clone()), &projects, &global);
        assert_eq!(
            resolved,
            Resolution {
                setting_tokens: setting,
                rule
            },
            "{conversation}"
        );
    }
}

/// A malformed project value reads as inherit too.
#[test]
fn a_malformed_project_setting_inherits() {
    let broken = project("prj_x", "x", json!({ "mode": "tokens", "tokens": "lots" }));
    let resolved = resolve_with(
        &metadata(json!({ "project_id": "prj_x" })),
        &[broken],
        &global(Some(300_000)),
    );
    assert_eq!(resolved.setting_tokens, Some(300_000));
    assert_eq!(resolved.rule, Rule::global());
}

/// A hand-written global `tokens` mode without valid tokens is off.
#[test]
fn a_global_tokens_mode_without_tokens_is_off() {
    let config = AutocompactConfig {
        mode: AutocompactMode::Tokens,
        tokens: None,
        instructions: Some("  keep the plan ".into()),
    };
    assert_eq!(global_level(&config), Level::Off);
    assert_eq!(
        global_instructions(&config).as_deref(),
        Some("keep the plan")
    );
    let off_with_tokens = AutocompactConfig {
        mode: AutocompactMode::Off,
        tokens: Some(200_000),
        instructions: None,
    };
    assert_eq!(global_level(&off_with_tokens), Level::Off);
}

/// Spec §3.2.1's table: `effective_tokens` and `cap_tokens`.
#[test]
fn the_numbers_follow_the_spec_table() {
    let opus = 116_000;
    let sonnet = 916_000;
    let rows = [
        (400_000, opus, None, 440_000),
        (400_000, sonnet, Some(400_000), 440_000),
        (160_000, opus, None, 180_000),
        (160_000, sonnet, Some(160_000), 180_000),
        (100_000, opus, Some(100_000), 120_000),
        (600_000, sonnet, Some(600_000), 660_000),
    ];
    for (setting, usable, effective, cap) in rows {
        assert_eq!(
            effective_tokens(Some(setting), usable),
            effective,
            "{setting} on {usable}"
        );
        assert_eq!(cap_tokens(setting), cap, "{setting}");
    }
    assert_eq!(effective_tokens(None, sonnet), None);
    // The floor: 10 % of 20k is 2k, the headroom is 20k.
    assert_eq!(cap_tokens(20_000), 40_000);
    assert_eq!(cap_tokens(2_000_000), 2_200_000);
    // Equal to usable is not below it.
    assert_eq!(effective_tokens(Some(116_000), 116_000), None);
}

#[test]
fn the_snapshot_carries_null_when_the_model_limit_comes_first() {
    let turn = TurnAutocompact::new(
        Resolution {
            setting_tokens: Some(160_000),
            rule: acme_rule(),
        },
        None,
    );
    let below = turn.snapshot(116_000);
    assert_eq!(
        serde_json::to_value(&below).unwrap(),
        json!({
            "setting_tokens": 160_000,
            "effective_tokens": null,
            "cap_tokens": 180_000,
            "rule": { "scope": "project", "project_id": "prj_acme", "project_name": "acme-web" },
        })
    );
    assert_eq!(turn.snapshot(916_000).effective_tokens, Some(160_000));
    let off = TurnAutocompact::new(
        Resolution {
            setting_tokens: None,
            rule: Rule::global(),
        },
        None,
    );
    assert_eq!(
        serde_json::to_value(off.snapshot(916_000)).unwrap(),
        json!({
            "setting_tokens": null,
            "effective_tokens": null,
            "cap_tokens": null,
            "rule": { "scope": "global" },
        })
    );
}

fn snapshot(total: u64, output: Option<u64>, usable: u64) -> ContextSnapshotV1 {
    serde_json::from_value(json!({
        "session_id": "s1",
        "turn_id": "t1",
        "step": 0,
        "model": "m",
        "usable": usable,
        "effective_max_output_tokens": 64_000,
        "total": total,
        "free": usable.saturating_sub(total),
        "categories": {
            "system_prompt": 0, "skills": 0, "tools": 0,
            "messages": { "user": 0, "assistant": 0, "function_result": 0, "custom": 0 },
            "overhead": 0, "hook_guidance": 0,
        },
        "compacted": false,
        "usage": output.map(|output| json!({ "input": total, "output": output })),
        "timestamp": 1,
    }))
    .expect("a snapshot")
}

#[test]
fn the_measure_adds_the_last_output() {
    assert_eq!(measure(&snapshot(150_000, Some(12_000), 916_000)), 162_000);
    assert_eq!(measure(&snapshot(150_000, None, 916_000)), 150_000);
}

/// The billed input decides, not the snapshot's `total`: before `exactify`
/// applies the billed numbers (a first turn, the first after a restart) the
/// total is context-manager's estimate, about 20 tokens a message high
/// (161,461 estimated against 157,622 billed with 186 messages).
#[test]
fn the_measure_is_the_billed_input_when_the_step_has_usage() {
    let mut estimated = snapshot(161_461, Some(1_200), 916_000);
    estimated.usage = Some(
        serde_json::from_value(
            json!({ "input": 7_622, "cache_read": 150_000, "cache_write": 0, "output": 1_200 }),
        )
        .unwrap(),
    );
    assert_eq!(measure(&estimated), 158_822);
    // No usage, or a usage with no input: the snapshot's total.
    let mut unbilled = snapshot(161_461, None, 916_000);
    assert_eq!(measure(&unbilled), 161_461);
    unbilled.usage = Some(serde_json::from_value(json!({ "output": 300 })).unwrap());
    assert_eq!(measure(&unbilled), 161_761);
}

/// D160: no budget without a setting, while paused, or while the back-off
/// holds; the cap otherwise.
#[test]
fn the_budget_is_the_cap_unless_paused_or_held() {
    let resolution = Resolution {
        setting_tokens: Some(160_000),
        rule: Rule::global(),
    };
    let mut turn = TurnAutocompact::new(resolution.clone(), None);
    assert_eq!(turn.budget(None), Some(180_000));
    turn.paused = true;
    assert_eq!(turn.budget(None), None);
    let held = TurnAutocompact::new(
        resolution,
        Some(Backoff {
            at_tokens: 170_000,
            reason: "failed".into(),
            at_ms: 1,
        }),
    );
    // Unknown size: held. Grown less than 10 % of 160k (16k): held.
    assert_eq!(held.budget(None), None);
    assert_eq!(
        held.budget(Some(&snapshot(180_000, Some(5_000), 916_000))),
        None
    );
    // 186k ≥ 170k + 16k: allowed again.
    assert_eq!(
        held.budget(Some(&snapshot(180_000, Some(6_000), 916_000))),
        Some(180_000)
    );
    let off = TurnAutocompact::new(
        Resolution {
            setting_tokens: None,
            rule: Rule::global(),
        },
        None,
    );
    assert_eq!(off.budget(None), None);
}

/// D166: a failure or a gain under 10 % sets the back-off; a usage limit,
/// a busy or a stopped attempt leaves it; a good gain clears it.
#[test]
fn the_back_off_follows_the_outcome() {
    let set = |status, usage_limit, after| backoff_after(status, usage_limit, 200_000, after, 7);
    let backoff = |reason: &str| {
        Some(Some(Backoff {
            at_tokens: 200_000,
            reason: reason.into(),
            at_ms: 7,
        }))
    };
    assert_eq!(set(Status::Ok, false, Some(40_000)), Some(None));
    // 181k is 90.5 % of 200k: a gain under 10 %.
    assert_eq!(set(Status::Ok, false, Some(181_000)), backoff("small_gain"));
    // Exactly 10 % is enough.
    assert_eq!(set(Status::Ok, false, Some(180_000)), Some(None));
    assert_eq!(set(Status::Failed, false, None), backoff("failed"));
    assert_eq!(set(Status::Failed, true, None), None);
    assert_eq!(set(Status::Empty, false, None), backoff("empty"));
    assert_eq!(set(Status::Busy, false, None), None);
    assert_eq!(set(Status::Cancelled, false, None), None);
    let held = Backoff {
        at_tokens: 200_000,
        reason: "failed".into(),
        at_ms: 7,
    };
    assert!(!held.allows(215_999, 160_000));
    assert!(held.allows(216_000, 160_000));
}

/// D167: typed codes, each with a message ready for the screen.
#[test]
fn an_override_is_checked_with_typed_codes() {
    assert_eq!(
        validate_override(&json!({ "mode": "inherit", "tokens": 160_000 })).unwrap(),
        json!({ "mode": "inherit" })
    );
    assert_eq!(
        validate_override(&json!({ "mode": "off" })).unwrap(),
        json!({ "mode": "off" })
    );
    assert_eq!(
        validate_override(&json!({ "mode": "tokens", "tokens": 20_000 })).unwrap(),
        json!({ "mode": "tokens", "tokens": 20_000 })
    );
    assert_eq!(
        validate_override(&json!({ "mode": "tokens", "tokens": 2_000_000 })).unwrap(),
        json!({ "mode": "tokens", "tokens": 2_000_000 })
    );
    let code = |value: Value| {
        validate_override(&value)
            .unwrap_err()
            .rejected_code()
            .unwrap()
    };
    assert_eq!(code(json!({ "mode": "sometimes" })), "bad_setting");
    assert_eq!(code(json!({ "mode": "tokens" })), "bad_setting");
    assert_eq!(code(json!({})), "bad_setting");
    assert_eq!(code(json!("off")), "bad_setting");
    assert_eq!(
        code(json!({ "mode": "tokens", "tokens": 19_999 })),
        "bad_tokens"
    );
    assert_eq!(
        code(json!({ "mode": "tokens", "tokens": 2_000_001 })),
        "bad_tokens"
    );
    assert_eq!(
        code(json!({ "mode": "tokens", "tokens": 1.5e5 })),
        "bad_tokens"
    );
    let HarnessError::Rejected(wire) =
        validate_override(&json!({ "mode": "tokens", "tokens": 1 })).unwrap_err()
    else {
        panic!("rejected")
    };
    let wire: Value = serde_json::from_str(&wire).unwrap();
    assert_eq!(wire["message"], "Between 20k and 2M tokens");
}

#[test]
fn the_global_setting_is_checked_with_typed_codes() {
    assert_eq!(
        validate_global(&json!({ "mode": "tokens", "tokens": 160_000, "instructions": "keep it" }))
            .unwrap(),
        AutocompactConfig {
            mode: AutocompactMode::Tokens,
            tokens: Some(160_000),
            instructions: Some("keep it".into()),
        }
    );
    assert_eq!(
        validate_global(&json!({})).unwrap(),
        AutocompactConfig::default()
    );
    let code = |value: Value| {
        validate_global(&value)
            .unwrap_err()
            .rejected_code()
            .unwrap()
    };
    assert_eq!(code(json!({ "mode": "inherit" })), "bad_setting");
    assert_eq!(code(json!({ "mode": "tokens" })), "bad_setting");
    assert_eq!(code(json!({ "extra": 1 })), "bad_setting");
    assert_eq!(code(json!({ "mode": "off", "tokens": 3 })), "bad_tokens");
    assert_eq!(
        code(json!({ "instructions": "x".repeat(1_001) })),
        "bad_instructions"
    );
    assert_eq!(code(json!({ "instructions": 5 })), "bad_instructions");
}

#[test]
fn sizes_and_rules_read_as_the_notices_write_them() {
    assert_eq!(short_tokens(160_000), "160k");
    assert_eq!(short_tokens(161_400), "161k");
    assert_eq!(short_tokens(1_500), "1.5k");
    assert_eq!(short_tokens(20_000), "20k");
    assert_eq!(short_tokens(1_200_000), "1.2M");
    assert_eq!(short_tokens(2_000_000), "2M");
    assert_eq!(short_tokens(900), "900");
    assert_eq!(Rule::global().name(), "global rule");
    assert_eq!(Rule::session().name(), "this conversation's rule");
    assert_eq!(acme_rule().name(), "acme-web rule");
}

#[test]
fn the_log_entry_keeps_the_contract_shape() {
    let entry = LogEntry {
        session_id: "s1".into(),
        title: "Fix the build".into(),
        at_ms: 5,
        trigger: crate::compaction::Trigger::Inline,
        threshold_tokens: Some(160_000),
        context_before: Some(181_000),
        context_after: Some(30_000),
        rule: Some(acme_rule()),
    };
    assert_eq!(
        serde_json::to_value(&entry).unwrap(),
        json!({
            "session_id": "s1",
            "title": "Fix the build",
            "at_ms": 5,
            "trigger": "inline",
            "threshold_tokens": 160_000,
            "context_before": 181_000,
            "context_after": 30_000,
            "rule": { "scope": "project", "project_id": "prj_acme", "project_name": "acme-web" },
        })
    );
}
