//! The worker's public surface: seven functions over one store.
//!
//! Every handler is async bus work (state reads and writes over the engine),
//! reading the live config snapshot per call so a configuration change takes
//! effect on the next invocation with no restart.

pub mod create;
pub mod delete;
pub mod family;
pub mod get;
pub mod list;
pub mod syntax;
pub mod update;
pub mod validate;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::configuration::ConfigCell;
use crate::store::Store;

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
        spec::<create::Request, create::Response>(create::ID, create::DESC),
        spec::<get::Request, get::Response>(get::ID, get::DESC),
        spec::<list::Request, list::Response>(list::ID, list::DESC),
        spec::<update::Request, update::Response>(update::ID, update::DESC),
        spec::<delete::Request, delete::Response>(delete::ID, delete::DESC),
        spec::<syntax::Request, syntax::Response>(syntax::ID, syntax::DESC),
        spec::<validate::Request, validate::Response>(validate::ID, validate::DESC),
    ]
}

/// Register one function whose handler reads the live config snapshot and the
/// shared store per call.
macro_rules! register_canvas {
    ($iii:expr, $cell:expr, $store:expr, $module:ident) => {{
        let cell = $cell.clone();
        let store = $store.clone();
        $iii.register_function(
            $module::ID,
            RegisterFunction::new_async(move |req: $module::Request| {
                let cell = cell.clone();
                let store = store.clone();
                async move {
                    let cfg = cell.read().await.clone();
                    $module::handle(&store, req, &cfg)
                        .await
                        .map_err(Error::Handler)
                }
            })
            .description($module::DESC),
        );
    }};
}

pub fn register_all(iii: &Arc<IIIClient>, cell: &ConfigCell, store: &Arc<Store>) {
    register_canvas!(iii, cell, store, create);
    register_canvas!(iii, cell, store, get);
    register_canvas!(iii, cell, store, list);
    register_canvas!(iii, cell, store, update);
    register_canvas!(iii, cell, store, delete);
    register_canvas!(iii, cell, store, syntax);
    register_canvas!(iii, cell, store, validate);
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
                "canvas::create",
                "canvas::get",
                "canvas::list",
                "canvas::update",
                "canvas::delete",
                "canvas::syntax",
                "canvas::validate",
            ]
        );
    }

    /// Function ids are the public wire surface: kebab-case in multi-word
    /// segments, never snake_case, and always under this worker's namespace.
    #[test]
    fn function_ids_follow_the_naming_rule() {
        for spec in catalog() {
            assert!(
                spec.function_id.starts_with("canvas::"),
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
