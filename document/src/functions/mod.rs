//! The worker's public surface: four functions over one converter.
//!
//! Three of the handlers are synchronous CPU work over an owned buffer, so each
//! runs on a blocking thread rather than on the async runtime. Conversion is
//! fast — single-digit milliseconds for a typical document — but a large
//! workbook is long enough to stall the executor and every other call sharing
//! it.
//!
//! `document::ocr` is the exception and registers differently: it spends its
//! time waiting on other workers rather than on this machine's CPU, so it stays
//! async and takes a [`Bus`] instead of a thread.

pub mod assets;
pub mod detect;
pub mod markdown;
pub mod ocr;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::bus::Bus;
use crate::configuration::ConfigCell;

/// One entry of the wire surface: what a caller sees for one function.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Build a schema the same way iii-sdk does at registration, so the snapshot
/// equals what actually ships.
fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req, Resp>(function_id: &'static str, description: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Golden-tested in
/// `tests/schemas.rs`; keep in lockstep with [`register_all`].
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<detect::Request, detect::Response>(detect::ID, detect::DESC),
        spec::<markdown::Request, markdown::Response>(markdown::ID, markdown::DESC),
        spec::<assets::Request, assets::Response>(assets::ID, assets::DESC),
        spec::<ocr::Request, ocr::Response>(ocr::ID, ocr::DESC),
    ]
}

/// Register one function whose handler is blocking CPU work over the live
/// config snapshot.
///
/// The snapshot is read per call, so a configuration change takes effect on the
/// next invocation with no restart and no re-registration.
macro_rules! register_blocking {
    ($iii:expr, $cell:expr, $module:ident) => {{
        let cell = $cell.clone();
        $iii.register_function(
            $module::ID,
            RegisterFunction::new_async(move |req: $module::Request| {
                let cell = cell.clone();
                async move {
                    let cfg = cell.read().await.clone();
                    tokio::task::spawn_blocking(move || $module::handle(req, &cfg))
                        .await
                        .map_err(|e| Error::Handler(format!("{} panicked: {e}", $module::ID)))?
                        .map_err(Error::Handler)
                }
            })
            .description($module::DESC),
        );
    }};
}

pub fn register_all(iii: &Arc<IIIClient>, cell: &ConfigCell, bus: Arc<dyn Bus>) {
    register_blocking!(iii, cell, detect);
    register_blocking!(iii, cell, markdown);
    register_blocking!(iii, cell, assets);

    // OCR waits on the browser and on a model rather than on this machine, so
    // it stays on the async runtime: a blocking thread would sit idle for the
    // whole call and there are only so many of them.
    let cell = cell.clone();
    iii.register_function(
        ocr::ID,
        RegisterFunction::new_async(move |req: ocr::Request| {
            let (cell, bus) = (cell.clone(), bus.clone());
            async move {
                let cfg = cell.read().await.clone();
                ocr::handle(req, cfg, bus).await.map_err(Error::Handler)
            }
        })
        .description(ocr::DESC),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_every_function_in_registration_order() {
        let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
        assert_eq!(
            ids,
            vec![
                "document::detect",
                "document::to-markdown",
                "document::extract-assets",
                "document::ocr",
            ]
        );
    }

    /// Function ids are the public wire surface: kebab-case in multi-word
    /// segments, never snake_case, and always under this worker's namespace.
    #[test]
    fn function_ids_follow_the_naming_rule() {
        for spec in catalog() {
            assert!(
                spec.function_id.starts_with("document::"),
                "{} is outside the worker namespace",
                spec.function_id
            );
            assert!(
                !spec.function_id.contains('_'),
                "{} uses snake_case; multi-word segments are kebab-case",
                spec.function_id
            );
            assert_eq!(
                spec.function_id.to_lowercase(),
                spec.function_id,
                "{} is not lowercase",
                spec.function_id
            );
        }
    }

    #[test]
    fn every_function_carries_a_description() {
        for spec in catalog() {
            assert!(
                spec.description.len() > 40,
                "{} needs a description a caller can act on",
                spec.function_id
            );
        }
    }
}
