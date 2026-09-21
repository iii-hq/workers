//! The registered function surface and its schema catalog.
//!
//! Internal handlers — queue steps, trigger targets, the configuration
//! doorbell — carry `internal: true` so they stay off the catalog agents
//! browse, and `trace_hidden: true` so their spans do not clutter the trace
//! views of the very pipeline they observe
//! (`docs/sops/trace-hidden-functions.md`).

use std::sync::Arc;

use iii_sdk::{IIIClient, RegisterFunction};
use schemars::{schema::RootSchema, JsonSchema};

use crate::configuration::{ConfigCell, ConfigErrorCell};
use crate::{
    ids, Counters, GroupCountsV1, InvestigationCountsV1, StatusRequestV1, StatusResponseV1,
};

pub const STATUS_ID: &str = "sentinel::status";
pub const STATUS_DESC: &str = "Health and counters: whether ingest is enabled, the state of the engine's telemetry stores, per-source ingest counters (including values redacted on capture and traces lost before capture), group and investigation counts, and which mapped repositories are present on this machine.";

/// The one write an investigation may make. Registered with the
/// investigation surface; named here because the deny lists and the
/// permission rules key off it.
pub const DIAGNOSIS_RECORD_ID: &str = "sentinel::diagnosis::record";

pub use crate::configuration::{CONFIG_CHANGE_DESC, CONFIG_CHANGE_ID};

pub struct Deps {
    pub config: ConfigCell,
    pub config_error: ConfigErrorCell,
    pub counters: Arc<Counters>,
}

pub fn register_all(iii: &IIIClient, deps: &Arc<Deps>) {
    let current = deps.clone();
    iii.register_function(
        STATUS_ID,
        RegisterFunction::new_async(move |_request: StatusRequestV1| {
            let deps = current.clone();
            async move { Ok::<_, iii_sdk::errors::Error>(status(&deps).await) }
        })
        .description(STATUS_DESC),
    );
}

async fn status(deps: &Deps) -> StatusResponseV1 {
    let config = deps.config.read().await.clone();
    let config_error = deps.config_error.read().await.clone();
    deps.counters.snapshot(
        &config,
        ids::now_ms(),
        GroupCountsV1::default(),
        InvestigationCountsV1::default(),
        config_error,
    )
}

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: RootSchema,
    pub response_schema: RootSchema,
}

fn schema_of<T: JsonSchema>() -> RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// Every function this worker registers, with its published schemas. Golden
/// tested: a surface change that is not deliberate fails the build.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<StatusRequestV1, StatusResponseV1>(STATUS_ID, STATUS_DESC),
        spec::<iii_config_client::OnConfigChangeEvent, iii_config_client::OnConfigChangeResponse>(
            CONFIG_CHANGE_ID,
            CONFIG_CHANGE_DESC,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkerConfig;
    use tokio::sync::RwLock;

    fn deps(config: WorkerConfig) -> Arc<Deps> {
        Arc::new(Deps {
            config: Arc::new(RwLock::new(Arc::new(config))),
            config_error: Arc::new(RwLock::new(None)),
            counters: Arc::new(Counters::default()),
        })
    }

    #[tokio::test]
    async fn status_reports_the_live_configuration_and_readiness() {
        let deps = deps(WorkerConfig::default());
        let before = status(&deps).await;
        assert!(!before.enabled);
        assert!(before.sources.trace);
        assert_eq!(before.ingest.redactions, 0);

        deps.counters.mark_ready();
        deps.counters.add_redactions(3);
        let after = status(&deps).await;
        assert!(after.enabled);
        assert_eq!(after.ingest.redactions, 3);
    }

    #[tokio::test]
    async fn status_names_the_field_that_refused_the_stored_configuration() {
        let deps = deps(WorkerConfig::default());
        deps.counters.mark_ready();
        *deps.config_error.write().await =
            Some("sentinel/invalid_request: workers_ttl_ms must be at least 1000".into());

        let status = status(&deps).await;
        assert!(!status.enabled, "a refused configuration disables ingest");
        assert!(status
            .config_error
            .is_some_and(|reason| reason.contains("workers_ttl_ms")));
    }

    #[tokio::test]
    async fn status_follows_a_configuration_swap() {
        let deps = deps(WorkerConfig::default());
        deps.counters.mark_ready();
        assert!(status(&deps).await.sources.log);

        {
            let mut live = deps.config.write().await;
            let mut next = WorkerConfig::default();
            next.sources.log.enabled = false;
            *live = Arc::new(next);
        }
        assert!(!status(&deps).await.sources.log);
    }
}
