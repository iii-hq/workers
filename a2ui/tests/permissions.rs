use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct PermissionFile {
    rules: Vec<Value>,
}

#[derive(Debug, PartialEq)]
enum Decision {
    Allow,
    Deny,
    NeedsApproval,
}

fn decision(rules: &[Value], function_id: &str) -> Decision {
    for rule in rules.iter().filter_map(Value::as_str) {
        let (deny, pattern) = rule
            .strip_prefix('!')
            .map_or((false, rule), |pattern| (true, pattern));
        if glob_matches(pattern, function_id) {
            return if deny {
                Decision::Deny
            } else {
                Decision::Allow
            };
        }
    }
    Decision::NeedsApproval
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return pattern == value;
    };
    value.starts_with(prefix) && value.ends_with(suffix)
}

#[test]
fn repository_policy_keeps_console_ingress_denied_and_public_a2ui_tools_allowed() {
    let policy: PermissionFile =
        serde_yaml::from_str(include_str!("../../iii-permissions.yaml")).unwrap();
    for function_id in [
        "a2ui::action",
        "a2ui::binding::apply",
        "a2ui::stamp-session",
        "a2ui::on-config-change",
    ] {
        assert_eq!(
            decision(&policy.rules, function_id),
            Decision::Deny,
            "{function_id}"
        );
    }
    for function_id in [
        "a2ui::generate",
        "a2ui::surface::apply",
        "a2ui::surface::patch",
        "a2ui::surface::export-code",
        "a2ui::binding::set",
        "a2ui::template::apply",
    ] {
        assert_eq!(
            decision(&policy.rules, function_id),
            Decision::Allow,
            "{function_id}"
        );
    }
}
