//! Browser telemetry settings for the console SPA.
//!
//! The browser has no process environment, so `GET /runtime` carries what
//! posthog-js needs: whether it may run at all, and which PostHog person its
//! events belong to. The engine decides: `iii-telemetry::settings` returns
//! the result of its own opt-out check, which also sees the engine config
//! (`enabled: false` on the telemetry worker), and the engine's `device_id`,
//! so console events join the same person as the engine and harness events.
//!
//! An engine without that function gets the local check below, which mirrors
//! the engine's environment checks (see
//! `engine/src/workers/telemetry/environment.rs`) but cannot see its config.

use iii_sdk::{protocol::TriggerRequest, IIIClient};
use serde_json::{json, Value};
use std::path::Path;

const SETTINGS_FN: &str = "iii-telemetry::settings";
const SETTINGS_TIMEOUT_MS: u64 = 2000;

/// The engine's public project key and host, overridable with the same
/// variables the engine reads.
const POSTHOG_PROJECT_API_KEY: &str = "phc_mmRHNXK6hkykVuxVp3JPn7R7sbo3ckSpEZLUKjofCWn6";
const POSTHOG_DEFAULT_HOST: &str = "https://us.i.posthog.com";

/// The engine's CI markers: any one set turns telemetry off.
const CI_ENV_VARS: &[&str] = &[
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "CIRCLECI",
    "JENKINS_URL",
    "TRAVIS",
    "BUILDKITE",
    "TF_BUILD",
    "CODEBUILD_BUILD_ID",
    "BITBUCKET_BUILD_NUMBER",
    "DRONE",
    "TEAMCITY_VERSION",
];

/// The `telemetry` object of `GET /runtime`: the engine's answer, or the
/// local check when the engine cannot give one.
pub async fn runtime(iii: Option<&IIIClient>) -> Value {
    let answer = match iii {
        Some(iii) => iii
            .trigger(
                TriggerRequest {
                    function_id: SETTINGS_FN.to_string(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(SETTINGS_TIMEOUT_MS),
                }
                .namespace("default"),
            )
            .await
            .map_err(|e| e.to_string()),
        None => Err("no engine client".to_string()),
    };
    from_engine(answer, local)
}

/// The engine's answer when it has a boolean `enabled`, else `fallback()`.
/// Only the fields the browser needs are passed on.
fn from_engine(answer: Result<Value, String>, fallback: impl FnOnce() -> Value) -> Value {
    let Ok(answer) = answer else {
        return fallback();
    };
    match answer.get("enabled").and_then(Value::as_bool) {
        Some(false) => json!({ "enabled": false }),
        Some(true) => json!({
            "enabled": true,
            "distinct_id": answer["distinct_id"],
            "api_key": answer["api_key"],
            "host": answer["host"],
        }),
        None => fallback(),
    }
}

/// The local check, read from this process.
fn local() -> Value {
    let iii_dir = std::env::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".iii");
    settings(|k| std::env::var(k).ok(), &iii_dir)
}

/// `env` reads a variable; `iii_dir` is `~/.iii`.
fn settings(env: impl Fn(&str) -> Option<String>, iii_dir: &Path) -> Value {
    if opted_out(&env, iii_dir) {
        return json!({ "enabled": false });
    }
    let distinct_id = std::fs::read_to_string(iii_dir.join("telemetry.yaml"))
        .ok()
        .and_then(|yaml| device_id(&yaml));
    json!({
        "enabled": true,
        "distinct_id": distinct_id,
        "api_key": env("POSTHOG_PROJECT_API_KEY")
            .or_else(|| env("POSTHOG_API_KEY"))
            .unwrap_or_else(|| POSTHOG_PROJECT_API_KEY.into()),
        "host": env("POSTHOG_HOST").unwrap_or_else(|| POSTHOG_DEFAULT_HOST.into()),
    })
}

fn opted_out(env: &impl Fn(&str) -> Option<String>, iii_dir: &Path) -> bool {
    let falsey = env("III_TELEMETRY_ENABLED").is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        )
    });
    falsey
        || CI_ENV_VARS.iter().any(|k| env(k).is_some())
        || env("III_TELEMETRY_DEV").as_deref() == Some("true")
        || iii_dir.join("telemetry_dev_optout").exists()
}

/// `identity.device_id` from the engine's `telemetry.yaml`.
fn device_id(yaml: &str) -> Option<String> {
    let doc: serde_yaml::Value = serde_yaml::from_str(yaml).ok()?;
    doc.get("identity")?
        .get("device_id")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.to_string())
        }
    }

    fn local_on() -> Value {
        json!({ "enabled": true, "distinct_id": "local", "api_key": "phc_local", "host": "h" })
    }

    #[test]
    fn the_engine_answer_wins_when_it_says_off() {
        let value = from_engine(Ok(json!({ "enabled": false })), local_on);
        assert_eq!(value, json!({ "enabled": false }));
    }

    #[test]
    fn the_engine_answer_wins_when_it_says_on() {
        let value = from_engine(
            Ok(json!({
                "enabled": true,
                "distinct_id": "dev-1",
                "api_key": "phc_engine",
                "host": "https://eu.i.posthog.com",
                "extra": "dropped",
            })),
            local_on,
        );
        assert_eq!(
            value,
            json!({
                "enabled": true,
                "distinct_id": "dev-1",
                "api_key": "phc_engine",
                "host": "https://eu.i.posthog.com",
            })
        );
    }

    #[test]
    fn an_engine_without_the_function_falls_back_to_the_local_check() {
        let value = from_engine(Err("function not found".into()), local_on);
        assert_eq!(value, local_on());
    }

    #[test]
    fn an_answer_without_enabled_falls_back_to_the_local_check() {
        for answer in [json!({}), json!(null), json!({ "enabled": "yes" })] {
            assert_eq!(from_engine(Ok(answer), local_on), local_on());
        }
    }

    #[test]
    fn telemetry_is_on_by_default_with_the_engine_key_and_device_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("telemetry.yaml"),
            "version: 2\nidentity:\n  device_id: dev-1\n",
        )
        .unwrap();
        let value = settings(env_of(&[]), dir.path());
        assert_eq!(value["enabled"], true);
        assert_eq!(value["distinct_id"], "dev-1");
        assert_eq!(value["api_key"], POSTHOG_PROJECT_API_KEY);
        assert_eq!(value["host"], POSTHOG_DEFAULT_HOST);
    }

    #[test]
    fn a_missing_device_id_leaves_the_person_to_posthog() {
        let dir = tempfile::tempdir().unwrap();
        let value = settings(env_of(&[]), dir.path());
        assert_eq!(value["enabled"], true);
        assert!(value["distinct_id"].is_null());
    }

    #[test]
    fn every_engine_opt_out_turns_telemetry_off() {
        let dir = tempfile::tempdir().unwrap();
        for env in [
            env_of(&[("III_TELEMETRY_ENABLED", "false")]),
            env_of(&[("III_TELEMETRY_ENABLED", " OFF ")]),
            env_of(&[("III_TELEMETRY_ENABLED", "0")]),
            env_of(&[("GITHUB_ACTIONS", "true")]),
            env_of(&[("III_TELEMETRY_DEV", "true")]),
        ] {
            assert_eq!(settings(env, dir.path()), json!({ "enabled": false }));
        }
        std::fs::write(dir.path().join("telemetry_dev_optout"), "").unwrap();
        assert_eq!(
            settings(env_of(&[]), dir.path()),
            json!({ "enabled": false })
        );
    }

    #[test]
    fn a_truthy_or_unknown_value_keeps_telemetry_on() {
        let dir = tempfile::tempdir().unwrap();
        for env in [
            env_of(&[("III_TELEMETRY_ENABLED", "true")]),
            env_of(&[("III_TELEMETRY_ENABLED", "maybe")]),
        ] {
            assert_eq!(settings(env, dir.path())["enabled"], true);
        }
    }

    #[test]
    fn the_engine_overrides_change_the_key_and_host() {
        let dir = tempfile::tempdir().unwrap();
        let value = settings(
            env_of(&[
                ("POSTHOG_API_KEY", "phc_x"),
                ("POSTHOG_HOST", "https://eu.i.posthog.com"),
            ]),
            dir.path(),
        );
        assert_eq!(value["api_key"], "phc_x");
        assert_eq!(value["host"], "https://eu.i.posthog.com");
    }
}
