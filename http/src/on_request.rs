//! `http::on-request`: one call per HTTP request.
//!
//! Each observer's args (`{ method, path, public_webhook?,
//! middleware_function_ids? }`, see [`HttpRouteArgs`]) declare one route. The
//! function's hooks keep the [`RouteTable`] the server matches requests
//! against, and deliver each request to its route:
//!
//! - **added** files the observer's route, or rejects the observer with
//!   `HTTP_ROUTE_CONFLICT` when another observer holds the same method and
//!   path shape;
//! - **removed** takes the route out;
//! - **match** picks the observer whose route the server matched: the server
//!   names it in the call's metadata ([`ROUTE_METADATA_KEY`]);
//! - **deliver** answers `404` when that observer is gone and `422` when its
//!   filters reject the request (`input` is the request), runs the route
//!   middleware, then calls the target through the observer. Its result is
//!   the call's result, which the server turns into the HTTP response.
//!
//! The handler returns the request it is called with: observers observe the
//! request, and the call's result is the route target's response. The
//! function is private: only this worker's server calls it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{
    AddedHook, Call, CallRequest, DeliverHook, IIIClient, Match, ObserveRejection, Observer,
    RegisterFunction, RemovedHook,
};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::configuration::ConfigCell;
use crate::middleware::{self, error_body, response_value, MiddlewareOutcome};
use crate::routes::{log_registered, log_unregistered, Route, RouteTable};
use crate::types::{HttpRequest, HttpRouteArgs};

/// The function observers declare routes on.
pub const ON_REQUEST_FN: &str = "http::on-request";

/// Rejection code of an observer whose route collides with a registered one.
pub const HTTP_ROUTE_CONFLICT: &str = "HTTP_ROUTE_CONFLICT";

/// Key of the call metadata that names the observer whose route the server
/// matched.
pub const ROUTE_METADATA_KEY: &str = "route";

pub const ON_REQUEST_DESCRIPTION: &str =
    "One call per HTTP request; observers observe the request. Observe it with args \
     { method, path } (plus optional public_webhook and middleware_function_ids) to serve \
     that route: each matching request calls the target with the HTTP request, and its \
     return value ({ status_code, headers, body }, or null after streaming to the response \
     channel) is the response. The observer's filters select requests by `input`; a request \
     they reject gets 422. Only the http worker calls it.";

/// Registers [`ON_REQUEST_FN`] with [`RouteHooks`] over `routes` as its
/// hooks. The hooks are set by id before the function registers, so they run
/// for every observer.
pub fn register(
    iii: &Arc<IIIClient>,
    routes: Arc<RwLock<RouteTable>>,
    config: ConfigCell,
) -> FunctionRef {
    let hooks = RouteHooks {
        routes,
        iii: iii.clone(),
        config,
    };
    iii.on_observer_added_for(ON_REQUEST_FN, hooks.clone());
    iii.on_observer_removed_for(ON_REQUEST_FN, hooks.clone());
    // Each call goes to the observer whose route the server matched.
    iii.on_observer_match_for(
        ON_REQUEST_FN,
        Match::keys(|obs| obs.id().into(), |call| matched_route(call).into()),
    );
    iii.on_observer_deliver_for(ON_REQUEST_FN, hooks);

    iii.register_function(
        ON_REQUEST_FN,
        RegisterFunction::new(|request: Value| Ok::<_, Error>(request))
            .description(ON_REQUEST_DESCRIPTION)
            .request_format(request_format())
            .response_format(request_format())
            .args_format::<HttpRouteArgs>()
            .private(),
    )
}

/// Removes the hooks [`register`] set, once the function is unregistered.
pub fn clear_hooks(iii: &IIIClient) {
    iii.clear_observer_added_for(ON_REQUEST_FN);
    iii.clear_observer_removed_for(ON_REQUEST_FN);
    iii.clear_observer_match_for(ON_REQUEST_FN);
    iii.clear_observer_deliver_for(ON_REQUEST_FN);
}

/// The observer id the server named in the call's metadata.
fn matched_route(call: &Call) -> Option<&str> {
    call.metadata.as_ref()?.get(ROUTE_METADATA_KEY)?.as_str()
}

/// The call's input and what observers observe: the [`HttpRequest`].
fn request_format() -> Value {
    serde_json::to_value(schemars::schema_for!(HttpRequest)).unwrap_or(Value::Null)
}

/// `404 NOT_FOUND`: the route's observer left after the server matched it.
fn not_found() -> Value {
    response_value(404, error_body("NOT_FOUND", "Not Found", None))
}

/// `422 CONDITION_NOT_MET`: the observer's filters rejected the request.
fn filters_not_met() -> Value {
    let mut body = error_body("CONDITION_NOT_MET", "Request condition not met", None);
    body["skipped"] = json!(true);
    response_value(422, body)
}

/// The hooks of [`ON_REQUEST_FN`]: one route per observer in the shared
/// [`RouteTable`], and delivery of each request to its route's observer.
#[derive(Clone)]
pub struct RouteHooks {
    pub routes: Arc<RwLock<RouteTable>>,
    iii: Arc<IIIClient>,
    /// Read per request, for the middleware and target timeouts.
    config: ConfigCell,
}

impl RouteHooks {
    /// The route `obs` asks for: its args, with a normalized path and an
    /// uppercased method.
    pub fn route_for(obs: &Observer) -> Result<Route, ObserveRejection> {
        let args: HttpRouteArgs = obs.args()?;
        Ok(Route {
            observer_id: obs.id().to_string(),
            http_path: args.path,
            http_method: args.method.to_uppercase(),
            public_webhook: args.public_webhook,
            middleware_function_ids: args.middleware_function_ids,
        })
    }
}

#[async_trait]
impl AddedHook for RouteHooks {
    async fn on_added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        let route = Self::route_for(&obs)?;
        self.routes
            .write()
            .await
            .insert(route.clone())
            .map_err(|message| ObserveRejection::new(HTTP_ROUTE_CONFLICT, message))?;
        log_registered(&route, obs.target());
        Ok(())
    }
}

#[async_trait]
impl RemovedHook for RouteHooks {
    async fn on_removed(&self, obs: Observer) {
        if let Some(route) = self.routes.write().await.remove(obs.id()) {
            log_unregistered(&route, obs.target());
        }
    }
}

#[async_trait]
impl DeliverHook for RouteHooks {
    async fn on_deliver(&self, call: Call, observers: Vec<Observer>) -> Result<Value, Error> {
        let Some(obs) = observers.into_iter().next() else {
            return Ok(not_found());
        };
        if !obs.matches(&call) {
            tracing::debug!(
                observer_id = %obs.id(),
                target = %obs.target(),
                "Observer filters rejected the request, skipping handler"
            );
            return Ok(filters_not_met());
        }
        let Some(route) = self.routes.read().await.get(obs.id()).cloned() else {
            return Ok(not_found());
        };
        let timeout_ms = self.config.read().await.default_timeout;
        let request = call.input;

        // Route middleware runs after the global middleware (in the server)
        // and before the target.
        for mw_fn_id in &route.middleware_function_ids {
            let input = middleware::middleware_input_for(&request);
            match middleware::execute_middleware(&self.iii, mw_fn_id, input, timeout_ms).await {
                Ok(MiddlewareOutcome::Continue) => {}
                Err(response) => return Ok(response),
            }
        }

        self.iii
            .call(CallRequest::observer(&obs, request).timeout(Duration::from_millis(timeout_ms)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_format_is_the_http_request() {
        let schema = request_format();
        for field in [
            "path",
            "method",
            "headers",
            "body",
            "request_body",
            "response",
        ] {
            assert!(
                schema["properties"].get(field).is_some(),
                "request_format lacks {field}"
            );
        }
    }

    #[test]
    fn the_matched_route_is_read_from_the_call_metadata() {
        let call = Call {
            metadata: Some(json!({ ROUTE_METADATA_KEY: "obs-1" })),
            ..Call::default()
        };
        assert_eq!(matched_route(&call), Some("obs-1"));
        assert_eq!(matched_route(&Call::default()), None);
        let other = Call {
            metadata: Some(json!({ "route": 7 })),
            ..Call::default()
        };
        assert_eq!(matched_route(&other), None);
    }

    #[test]
    fn hook_answers_are_responses() {
        let missing = crate::types::HttpResponse::from_function_return(not_found());
        assert_eq!(missing.status_code, 404);
        assert_eq!(missing.body["error"]["code"], "NOT_FOUND");
        let rejected = crate::types::HttpResponse::from_function_return(filters_not_met());
        assert_eq!(rejected.status_code, 422);
        assert_eq!(rejected.body["error"]["code"], "CONDITION_NOT_MET");
        assert_eq!(rejected.body["skipped"], true);
    }
}
