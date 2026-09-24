//! Integration with the `configuration` worker: register the schema, fetch
//! the authoritative value at boot, and hot-reload it in place.
//!
//! Configuration is a **required** boot dependency — the worker retries until
//! it answers rather than starting on guessed settings, because the settings
//! decide which projects an investigation may read.
//!
//! Reloads are serialized by [`iii_config_client::Reload`] with the fetch
//! inside the lock, so two overlapping `configuration:updated` deliveries
//! converge on the newest stored value instead of racing. A value that fails
//! to parse or validate is refused and the last good one stays live: a typo
//! in the console must not take the monitor down.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use iii_config_client::{self as config_client, EntrySpec, Reload};
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::schema_for;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::{SentinelError, WorkerConfig};

/// The live configuration, swapped whole on reload. Handlers take a read
/// guard, clone the inner `Arc` out, and drop the guard before doing work.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

/// Why the stored configuration was refused, when it was. `None` means the
/// live configuration is the operator's own.
pub type ConfigErrorCell = Arc<RwLock<Option<String>>>;

pub const DEFAULT_CONFIG_ID: &str = "sentinel";
/// Stable console form family. Unlike the entry id, it never changes when a
/// supervisor names this instance's entry.
pub const CONFIG_FORM_ID: &str = "sentinel";
pub const CONFIG_CHANGE_ID: &str = "sentinel::on-config-change";
pub const CONFIG_CHANGE_DESC: &str =
    "Internal configuration doorbell: re-reads the authoritative sentinel entry and swaps the live settings.";

const RETRY_BACKOFF_MS: u64 = 250;

/// This instance's configuration entry id. `III_CONFIG_NAME` when a
/// supervisor set one: a worker that hardcodes its id turns that id into a
/// scarce global name, and two instances would take turns overwriting one
/// entry.
pub fn config_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_string())
    })
    .as_str()
}

fn entry_spec() -> EntrySpec {
    EntrySpec {
        id: config_id(),
        form_id: CONFIG_FORM_ID,
        name: "Sentinel",
        description:
            "Error monitoring: sources, value redaction, fingerprint identity numbers, evidence retention, mapped projects, and the model an investigation opens with.",
        schema: serde_json::to_value(schema_for!(WorkerConfig))
            .expect("the configuration schema must serialize"),
        default_value: serde_json::to_value(WorkerConfig::default())
            .expect("the shipped configuration must serialize"),
    }
}

/// Register the schema and read the stored value back. The shipped defaults
/// are seeded only when nothing is stored yet.
pub async fn register_and_fetch(iii: &IIIClient) -> Result<WorkerConfig, SentinelError> {
    let spec = entry_spec();
    config_client::ensure(iii, &spec, None)
        .await
        .map_err(SentinelError::dependency)?;
    let stored = config_client::fetch(iii, spec.id)
        .await
        .map_err(SentinelError::dependency)?;
    parse(stored)
}

/// Block until the configuration worker answers, and report what came back.
///
/// The two failures are deliberately not the same. A configuration worker
/// that is down or slow is **transient**: retry, because there is nothing to
/// fix and the value is still out there. A stored value that does not parse
/// or validate is **permanent**: retrying it forever would read exactly like
/// the first case while the operator waits for a worker that will never
/// start. So a refused value boots on the shipped defaults, disabled, with
/// the reason in `sentinel::status` — and the reload doorbell heals it the
/// moment the entry is fixed.
pub async fn register_and_fetch_until_ready(iii: &IIIClient) -> (WorkerConfig, Option<String>) {
    loop {
        match register_and_fetch(iii).await {
            Ok(config) => return (config, None),
            Err(error) if is_permanent(&error) => {
                let reason = error.to_string();
                tracing::warn!(
                    %error,
                    "the stored sentinel configuration was refused; starting disabled on the \
                     shipped defaults until it is fixed"
                );
                return (WorkerConfig::default(), Some(reason));
            }
            Err(error) => {
                tracing::warn!(%error, "sentinel configuration unavailable; retrying");
                tokio::time::sleep(Duration::from_millis(RETRY_BACKOFF_MS)).await;
            }
        }
    }
}

/// A value the operator has to change, as opposed to a dependency that has
/// to come back.
fn is_permanent(error: &SentinelError) -> bool {
    matches!(error, SentinelError::InvalidRequest(_))
}

/// Parse a stored value. Nothing stored yet reads as the shipped defaults —
/// every field has one, so an empty entry is a valid idle worker.
fn parse(stored: Option<Value>) -> Result<WorkerConfig, SentinelError> {
    let value = stored.unwrap_or_else(|| json!({}));
    let mut config: WorkerConfig = serde_json::from_value(value).map_err(|error| {
        SentinelError::invalid(format!("stored sentinel configuration is invalid: {error}"))
    })?;
    config.fold_former_keys()?;
    config.validate()?;
    Ok(config)
}

/// Register the reload handler and bind it to this entry's updates. Run the
/// returned [`Reload`] once afterwards to close the boot gap: an update that
/// landed between the boot fetch and this binding fired into nothing.
pub fn bind_reload(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    config_error: ConfigErrorCell,
) -> Result<Reload, Error> {
    let client = iii.clone();
    config_client::on_change(
        iii,
        config_id(),
        CONFIG_CHANGE_ID,
        CONFIG_CHANGE_DESC,
        move || {
            let client = client.clone();
            let cell = cell.clone();
            let config_error = config_error.clone();
            async move {
                apply_latest(&client, &cell, &config_error).await;
            }
        },
    )
}

async fn apply_latest(iii: &IIIClient, cell: &ConfigCell, config_error: &ConfigErrorCell) {
    let stored = match config_client::fetch(iii, config_id()).await {
        Ok(stored) => stored,
        Err(error) => {
            tracing::warn!(%error, "could not re-read the sentinel configuration; keeping the live one");
            return;
        }
    };
    match parse(stored) {
        Ok(config) => {
            let mut live = cell.write().await;
            if live.database != config.database {
                tracing::warn!(
                    from = %live.database,
                    to = %config.database,
                    "the database connection changed; restart the worker to move the store"
                );
            }
            *live = Arc::new(config);
            *config_error.write().await = None;
            tracing::info!("sentinel configuration reloaded");
        }
        Err(error) => {
            // The live configuration stays; the reason becomes visible in
            // `sentinel::status` so a refused edit is not silent.
            *config_error.write().await = Some(error.to_string());
            tracing::warn!(%error, "refused an invalid sentinel configuration; the last good one stays live");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_reads_as_the_shipped_defaults() {
        let config = parse(None).expect("an unseeded entry is valid");
        assert_eq!(config, WorkerConfig::default());
    }

    #[test]
    fn a_partial_entry_keeps_the_defaults_it_does_not_mention() {
        let config = parse(Some(json!({ "enabled": false }))).expect("a partial entry is valid");
        assert!(!config.enabled);
        assert_eq!(config.database, "primary");
    }

    #[test]
    fn an_invalid_entry_is_refused_rather_than_applied() {
        let error = parse(Some(json!({ "workers_ttl_ms": 10 })))
            .expect_err("a value outside its bounds is refused");
        assert!(error.to_string().contains("workers_ttl_ms"));
    }

    #[test]
    fn a_broken_stored_value_is_permanent_and_a_dead_dependency_is_not() {
        assert!(is_permanent(&SentinelError::invalid("workers_ttl_ms")));
        assert!(!is_permanent(&SentinelError::dependency(
            "configuration::get timed out"
        )));
    }

    #[test]
    fn the_registration_separates_the_entry_id_from_the_form_family() {
        let spec = entry_spec();
        assert_eq!(spec.form_id, CONFIG_FORM_ID);
        assert!(spec.schema["properties"]["projects"].is_object());
        // The former name stays in the schema, so a stored value that still
        // uses it validates until it is saved under the new one — and it is
        // never required, or no value without it could be saved.
        assert!(spec.schema["properties"]["repositories"].is_object());
        assert!(
            spec.schema
                .get("required")
                .is_none_or(|required| required.as_array().is_some_and(|fields| fields.is_empty())),
            "no field is required: every one has a default ({})",
            spec.schema["required"]
        );
        assert!(spec.default_value["retention"]["cron"].is_string());
    }
}
