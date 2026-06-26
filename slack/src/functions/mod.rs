//! Slack Web API exposed as `slack::*` iii functions.
//!
//! Most methods are thin passthroughs: a typed request struct (so the console
//! renders a form and the agent gets typed params) is serialized straight into
//! the Slack API params and POSTed by [`crate::clients::slack`]. The
//! [`slack_method!`] macro generates one registration fn per method. Methods
//! that need bespoke logic (file upload, user-token search, config status, the
//! generic escape hatch) are written by hand.

pub mod admin;
pub mod assistant;
pub mod bindings;
pub mod call;
pub mod chat;
pub mod conversations;
pub mod events;
pub mod files;
pub mod interactions;
pub mod notify;
pub mod pins;
pub mod reactions;
pub mod search;
pub mod users;
pub mod views;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::deps::Deps;

/// Register a typed async function with the engine. Shared by hand-written
/// handlers and the [`slack_method!`] macro.
pub(crate) fn register<Req, Resp, F, Fut>(
    iii: &Arc<IIIClient>,
    deps: &Arc<Deps>,
    id: &str,
    description: &str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, Error>> + Send + 'static,
{
    let deps = deps.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let deps = deps.clone();
            let handler = handler.clone();
            async move { handler(deps, req).await }
        })
        .description(description),
    );
}

/// Generate a passthrough registration fn: serialize `$req` into Slack params,
/// POST `$method` with the bot token, return the full Slack payload.
#[macro_export]
macro_rules! slack_method {
    ($fname:ident, $id:expr, $method:expr, $desc:expr, $req:ty) => {
        pub fn $fname(
            iii: &::std::sync::Arc<::iii_sdk::IIIClient>,
            deps: &::std::sync::Arc<$crate::deps::Deps>,
        ) {
            $crate::functions::register::<$req, $crate::response::SlackResponse, _, _>(
                iii,
                deps,
                $id,
                $desc,
                |d, req: $req| async move {
                    let params = ::serde_json::to_value(&req).map_err(|e| {
                        ::iii_sdk::errors::Error::Handler(format!("serialize {}: {e}", $method))
                    })?;
                    let value = $crate::clients::slack::call(&d, $method, params).await?;
                    Ok($crate::response::SlackResponse::from_value(value))
                },
            );
        }
    };
}

pub fn register_all(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    chat::register(iii, deps);
    conversations::register(iii, deps);
    reactions::register(iii, deps);
    files::register(iii, deps);
    users::register(iii, deps);
    views::register(iii, deps);
    pins::register(iii, deps);
    search::register(iii, deps);
    assistant::register(iii, deps);
    admin::register(iii, deps);
    call::register(iii, deps);
    // Bridge functions (harmless when the harness stack is absent; their sibling
    // triggers simply fail to bind and the HTTP routes stay unregistered).
    notify::register(iii, deps);
    events::register(iii, deps);
    interactions::register(iii, deps);
    bindings::register(iii, deps);
    tracing::info!("all slack functions registered");
}
