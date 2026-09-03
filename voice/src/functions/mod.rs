//! The worker's public surface: ten functions over one engine, one session
//! registry and one speaker.
//!
//! Every handler reads the live config snapshot per call so a configuration
//! change takes effect on the next invocation with no restart; the one thing
//! that does reload is the recognizer itself, when the model or endpointing
//! rules change.

pub mod catalog;
pub mod dictation;
pub mod doctor;
pub mod speak;
pub mod transcribe;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::configuration::ConfigCell;
use crate::engine::Engine;
use crate::events::Emitter;
use crate::session::Sessions;
use crate::tts::Speaker;

/// Everything a handler can reach.
pub struct AppState {
    pub cfg: ConfigCell,
    pub engine: Arc<Engine>,
    pub sessions: Arc<Sessions>,
    pub speaker: Arc<Speaker>,
    pub emitter: Arc<Emitter>,
}

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

/// The full wire-surface catalog, in registration order. Keep in lockstep
/// with [`register_all`].
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<dictation::StartRequest, dictation::StartResponse>(
            dictation::START_ID,
            dictation::START_DESC,
        ),
        spec::<dictation::PushRequest, dictation::PushResponse>(
            dictation::PUSH_ID,
            dictation::PUSH_DESC,
        ),
        spec::<dictation::StopRequest, dictation::StopResponse>(
            dictation::STOP_ID,
            dictation::STOP_DESC,
        ),
        spec::<dictation::ListRequest, dictation::ListResponse>(
            dictation::LIST_ID,
            dictation::LIST_DESC,
        ),
        spec::<transcribe::Request, transcribe::Response>(transcribe::ID, transcribe::DESC),
        spec::<speak::Request, speak::Response>(speak::ID, speak::DESC),
        spec::<speak::StopRequest, speak::StopResponse>(speak::STOP_ID, speak::STOP_DESC),
        spec::<catalog::ListRequest, catalog::ListResponse>(catalog::LIST_ID, catalog::LIST_DESC),
        spec::<catalog::DownloadRequest, catalog::DownloadResponse>(
            catalog::DOWNLOAD_ID,
            catalog::DOWNLOAD_DESC,
        ),
        spec::<catalog::RemoveRequest, catalog::RemoveResponse>(
            catalog::REMOVE_ID,
            catalog::REMOVE_DESC,
        ),
        spec::<doctor::Request, doctor::Response>(doctor::ID, doctor::DESC),
    ]
}

macro_rules! register {
    ($iii:expr, $state:expr, $id:expr, $desc:expr, $handler:path) => {{
        let state = $state.clone();
        $iii.register_function(
            $id,
            RegisterFunction::new_async(move |req| {
                let state = state.clone();
                async move { $handler(&state, req).await.map_err(Error::Handler) }
            })
            .description($desc),
        );
    }};
}

/// Register every public function.
pub fn register_all(iii: &Arc<IIIClient>, state: &Arc<AppState>) {
    register!(
        iii,
        state,
        dictation::START_ID,
        dictation::START_DESC,
        dictation::start
    );
    register!(
        iii,
        state,
        dictation::PUSH_ID,
        dictation::PUSH_DESC,
        dictation::push
    );
    register!(
        iii,
        state,
        dictation::STOP_ID,
        dictation::STOP_DESC,
        dictation::stop
    );
    register!(
        iii,
        state,
        dictation::LIST_ID,
        dictation::LIST_DESC,
        dictation::list
    );
    register!(
        iii,
        state,
        transcribe::ID,
        transcribe::DESC,
        transcribe::handle
    );
    register!(iii, state, speak::ID, speak::DESC, speak::handle);
    register!(iii, state, speak::STOP_ID, speak::STOP_DESC, speak::stop);
    register!(
        iii,
        state,
        catalog::LIST_ID,
        catalog::LIST_DESC,
        catalog::list
    );
    register!(
        iii,
        state,
        catalog::DOWNLOAD_ID,
        catalog::DOWNLOAD_DESC,
        catalog::download
    );
    register!(
        iii,
        state,
        catalog::REMOVE_ID,
        catalog::REMOVE_DESC,
        catalog::remove
    );
    register!(iii, state, doctor::ID, doctor::DESC, doctor::handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_function_id_is_namespaced_and_unique() {
        let ids: Vec<&str> = catalog().iter().map(|s| s.function_id).collect();
        assert_eq!(ids.len(), 11);
        for id in &ids {
            assert!(id.starts_with("voice::"), "{id}");
        }
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn descriptions_are_prose() {
        for s in catalog() {
            assert!(
                s.description.len() > 40,
                "{} description too short",
                s.function_id
            );
            assert!(
                s.request_schema.schema.object.is_some()
                    || s.request_schema.schema.instance_type.is_some()
            );
        }
    }
}
