//! Browser telemetry settings for the console SPA.
//!
//! The browser has no process environment, so `GET /runtime` carries what
//! posthog-js needs: whether it may run at all, and which PostHog person its
//! events belong to. The opt-out mirrors the engine's own check (see
//! `engine/src/workers/telemetry/environment.rs`) so the console and the
//! engine always agree, and the person is the engine's `device_id`, so console
//! events join the same person as the engine and harness events.

use serde_json::{json, Value};
use std::path::Path;

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

/// The `telemetry` object of `GET /runtime`, read from this process.
pub fn runtime() -> Value {
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
