//! `http::on-request`: the observable function next to the `http` trigger
//! type (iii 0.25).
//!
//! The function is owner-delivered. Each observer's args (`{ method, path,
//! public_webhook?, middleware_function_ids?, condition_function_id? }`, see
//! [`HttpRouteArgs`]) declare one route; [`RouteObserver`] is its
//! [`ObserveHandler`] and files that route in the same [`RouteTable`] the
//! `http` trigger bindings use, with the observer as its
//! [`RouteSink::Observer`]. The server then serves it like any route: the
//! request goes through the same matching, middleware, condition and
//! channels, and the target is invoked with `obs.call`, whose result is the
//! HTTP response. The observer's filters are evaluated in-process against the
//! request (as `input`); a request they reject is answered `422`, like an
//! unmet condition.
//!
//! The function itself is never invoked: requests reach observers through the
//! server, so its handler refuses every direct invocation (`FORBIDDEN`).
//!
//! On an engine without observe (0.24.x) the SDK registers it as a plain
//! function and its hooks never run; the `http` trigger type carries every
//! route there.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::trigger::{log_registered, log_unregistered, Route, RouteSink, RouteTable};
use crate::types::{HttpRequest, HttpRouteArgs};

/// The function observers bind routes on.
pub const ON_REQUEST_FN: &str = "http::on-request";

/// Rejection code of an observer whose route collides with a registered one.
pub const HTTP_ROUTE_CONFLICT: &str = "HTTP_ROUTE_CONFLICT";

/// Error code of a direct invocation of [`ON_REQUEST_FN`].
pub const FORBIDDEN: &str = "FORBIDDEN";

pub const ON_REQUEST_DESCRIPTION: &str =
    "One event per HTTP request. Observe it with args { method, path } (plus optional \
     public_webhook, middleware_function_ids and the deprecated condition_function_id) to \
     serve that route: each matching request invokes the target with the HTTP request, and \
     its return value ({ status_code, headers, body }, or null after streaming to the \
     response channel) is the response. Filters are evaluated against the request as \
     `input`; a request they reject gets 422. Only the http worker routes requests: \
     invoking it directly is refused.";

/// Registers [`ON_REQUEST_FN`] with a [`RouteObserver`] over `routes` as its
/// hooks. `routes` is the table the `http` trigger bindings fill, so both
/// kinds of routes share one namespace and one conflict rule.
///
/// No `legacy_trigger_type`: this worker still provides the `http` trigger
/// type itself, which wins over the engine's legacy shim anyway.
pub fn register(iii: &IIIClient, routes: Arc<RwLock<RouteTable>>) -> FunctionRef {
    iii.register_function(
        ON_REQUEST_FN,
        RegisterFunction::new(|_request: Value| Err::<Value, _>(direct_invocation_error()))
            .description(ON_REQUEST_DESCRIPTION)
            .request_format(request_format())
            .response_format(response_format())
            .observe(
                Observe::owner()
                    .args_format::<HttpRouteArgs>()
                    .handler(RouteObserver { routes }),
            ),
    )
}

fn direct_invocation_error() -> Error {
    Error::Remote {
        code: FORBIDDEN.to_string(),
        message: format!(
            "{ON_REQUEST_FN} is an event source: observe it \
             (iii.observe({{ function_id: '{ON_REQUEST_FN}', args: {{ method, path }} }}, target)); \
             only the http worker routes requests to its observers"
        ),
        stacktrace: None,
    }
}

/// What each target receives: the same [`HttpRequest`] an `http` trigger's
/// function receives.
fn request_format() -> Value {
    serde_json::to_value(schemars::schema_for!(HttpRequest)).unwrap_or(Value::Null)
}

/// What each target returns, as `HttpResponse::from_function_return` reads it.
fn response_format() -> Value {
    json!({
        "type": ["object", "null"],
        "properties": {
            "status_code": { "type": "integer", "default": 200 },
            "headers": {
                "description": "An object of header values, or a list of \"Name: value\" lines",
                "type": ["object", "array"]
            },
            "body": { "description": "JSON by default; a string or raw value with a non-JSON content-type" }
        }
    })
}

/// The hooks of [`ON_REQUEST_FN`]: one route per observer, in the shared
/// [`RouteTable`].
#[derive(Clone)]
pub struct RouteObserver {
    pub routes: Arc<RwLock<RouteTable>>,
}

impl RouteObserver {
    /// The route `obs` asks for: its args, validated like an `http` trigger's
    /// config (normalized path, uppercased method).
    pub fn route_for(obs: &Observer) -> Result<Route, ObserveRejection> {
        let args: HttpRouteArgs = obs.args()?;
        Ok(Route {
            sink: RouteSink::Observer(obs.clone()),
            function_id: obs.target().to_string(),
            http_path: args.path,
            http_method: args.method.to_uppercase(),
            public_webhook: args.public_webhook,
            condition_function_id: args.condition_function_id,
            middleware_function_ids: args.middleware_function_ids,
        })
    }
}

#[async_trait]
impl ObserveHandler for RouteObserver {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        let route = Self::route_for(obs)?;
        self.routes
            .write()
            .await
            .insert(route.clone())
            .map_err(|message| ObserveRejection::new(HTTP_ROUTE_CONFLICT, message))?;
        log_registered(&route);
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        if let Some(route) = self.routes.write().await.remove_by_observer_id(obs.id()) {
            log_unregistered(&route);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_invocation_is_forbidden() {
        match direct_invocation_error() {
            Error::Remote { code, message, .. } => {
                assert_eq!(code, FORBIDDEN);
                assert!(message.contains(ON_REQUEST_FN));
            }
            other => panic!("expected a remote error, got {other:?}"),
        }
    }

    #[test]
    fn request_format_is_the_trigger_payload() {
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
}
