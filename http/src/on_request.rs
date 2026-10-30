//! `http::on-request`: one call per HTTP request.
//!
//! Each observer's args (`{ method, path, public_webhook?,
//! middleware_function_ids? }`, see [`HttpRouteArgs`]) declare one route. The
//! server calls the function for every request it receives (the CORS layer
//! answers `OPTIONS`), with the request as the server received it
//! ([`InboundRequest`]) as input. The function's hooks keep the
//! [`RouteTable`] and deliver each call to its route:
//!
//! - **added** files the observer's route, or rejects the observer with
//!   `HTTP_ROUTE_CONFLICT` when another observer holds the same method and
//!   path shape;
//! - **removed** takes the route out;
//! - **deliver** matches the request's method and path to a route. It
//!   answers `404` when no route of an active observer has the path, and
//!   `405` with `Allow` when routes have the path on other methods. A
//!   matched request gets `413` when its body is over the server's limit and
//!   `422` when the observer's filters reject the call. Then the global
//!   middleware and the route's middleware run, the hook opens the
//!   request-body and response channels, and it calls the target through the
//!   observer with the [`HttpRequest`]. The target's result is the call's
//!   result, which the server turns into the HTTP response.
//!
//! The deliver hook runs in the function's invocation context: the
//! middleware, the channel calls and the call through the observer have
//! `http::on-request` as their caller. The handler returns the request it is
//! called with. The function is private: only this worker's server calls it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::channel::StreamChannelRef;
use iii_sdk::errors::Error;
use iii_sdk::helpers::create_channel;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{
    AddedHook, Call, CallRequest, DeliverHook, IIIClient, ObserveRejection, Observer,
    RegisterFunction, RemovedHook,
};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::configuration::ConfigCell;
use crate::inflight::{InFlight, InFlightRequests, REQUEST_ID_KEY};
use crate::middleware::{self, error_body, response_value, MiddlewareOutcome};
use crate::routes::{log_registered, log_unregistered, Route, RouteTable};
use crate::types::{HttpRequest, HttpRouteArgs, InboundRequest};

/// The function observers declare routes on.
pub const ON_REQUEST_FN: &str = "http::on-request";

/// Rejection code of an observer whose route collides with a registered one.
pub const HTTP_ROUTE_CONFLICT: &str = "HTTP_ROUTE_CONFLICT";

/// Channel buffer size for both request-body and response channels (matches
/// the engine default in `views.rs`).
const CHANNEL_BUFFER: usize = 64;

pub const ON_REQUEST_DESCRIPTION: &str =
    "One call per HTTP request the http worker receives (OPTIONS requests are answered by its \
     CORS layer). Observe it with args { method, path } (plus optional public_webhook and \
     middleware_function_ids) to serve that route: the worker calls the target with the \
     HttpRequest (path template, path_params, query_params, headers, body, and the \
     request_body and response channels), and its return value ({ status_code, headers, \
     body }, or null after streaming to the response channel) is the response. A request \
     without a route gets 404 (405 when the path has routes on other methods); a request the \
     observer's filters reject gets 422. Filters read the request as the server received it: \
     input.method, input.path, input.query_params, input.headers, input.body. Only the http \
     worker calls it.";

/// Registers [`ON_REQUEST_FN`] with [`RouteHooks`] over `routes` as its
/// hooks. The hooks are set by id before the function registers, so they run
/// for every observer.
pub fn register(
    iii: &Arc<IIIClient>,
    routes: Arc<RwLock<RouteTable>>,
    requests: InFlightRequests,
    config: ConfigCell,
) -> FunctionRef {
    let hooks = RouteHooks {
        routes,
        requests,
        iii: iii.clone(),
        config,
    };
    iii.on_observer_added_for(ON_REQUEST_FN, hooks.clone());
    iii.on_observer_removed_for(ON_REQUEST_FN, hooks.clone());
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
    iii.clear_observer_deliver_for(ON_REQUEST_FN);
}

/// The id of the request in flight the server named in the call's metadata.
fn request_id(call: &Call) -> Option<&str> {
    call.metadata.as_ref()?.get(REQUEST_ID_KEY)?.as_str()
}

/// The call's input and output: the [`InboundRequest`].
fn request_format() -> Value {
    serde_json::to_value(schemars::schema_for!(InboundRequest)).unwrap_or(Value::Null)
}

/// `404 NOT_FOUND`: no route of an active observer has the path.
fn not_found() -> Value {
    response_value(404, error_body("NOT_FOUND", "Not Found", None))
}

/// `405 METHOD_NOT_ALLOWED`, with an `Allow` header listing the methods
/// routes have on the path.
fn method_not_allowed(allowed: &[String]) -> Value {
    let mut response = response_value(
        405,
        error_body("METHOD_NOT_ALLOWED", "Method Not Allowed", None),
    );
    response["headers"] = json!({ "allow": allowed.join(", ") });
    response
}

/// `413 PAYLOAD_TOO_LARGE`: the body is over the server's limit.
fn payload_too_large() -> Value {
    response_value(
        413,
        error_body("PAYLOAD_TOO_LARGE", "Request body too large", None),
    )
}

/// `422 CONDITION_NOT_MET`: the observer's filters rejected the request.
fn filters_not_met() -> Value {
    let mut body = error_body("CONDITION_NOT_MET", "Request condition not met", None);
    body["skipped"] = json!(true);
    response_value(422, body)
}

/// `500 INTERNAL_ERROR` with `message`.
fn internal_error(message: &str) -> Value {
    response_value(500, error_body("INTERNAL_ERROR", message, None))
}

/// The hooks of [`ON_REQUEST_FN`]: one route per observer in the shared
/// [`RouteTable`], and delivery of each request to its route's observer.
#[derive(Clone)]
pub struct RouteHooks {
    pub routes: Arc<RwLock<RouteTable>>,
    requests: InFlightRequests,
    iii: Arc<IIIClient>,
    /// Read per call, for the global middleware and the timeouts.
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

    /// The route `method` and `path` match on the listener, with its path
    /// params, or the response for a request without one.
    async fn match_route(
        &self,
        method: &str,
        path: &str,
        public_webhooks_only: bool,
    ) -> Result<(Route, HashMap<String, String>), Value> {
        let routes = self.routes.read().await;
        if let Some(matched) = routes.match_route_for_listener(method, path, public_webhooks_only) {
            return Ok(matched);
        }
        let allowed = routes.allowed_methods_for_listener(path, public_webhooks_only);
        Err(if allowed.is_empty() {
            not_found()
        } else {
            method_not_allowed(&allowed)
        })
    }

    /// Opens the request-body and response channels for a target about to
    /// run: writes the request's bytes into the first and hands the second's
    /// reader to the server. Returns the refs the target receives, or the
    /// response for a failure.
    async fn open_channels(
        &self,
        request: &InFlight,
    ) -> Result<(StreamChannelRef, StreamChannelRef), Value> {
        let request_body = create_channel(&self.iii, Some(CHANNEL_BUFFER))
            .await
            .map_err(|e| internal_error(&format!("failed to create request channel: {e}")))?;
        let response = create_channel(&self.iii, Some(CHANNEL_BUFFER))
            .await
            .map_err(|e| internal_error(&format!("failed to create response channel: {e}")))?;

        // The target reads the body concurrently; the writer closes the
        // channel so the target's reader sees EOF.
        let bytes = request.body().cloned().unwrap_or_default();
        let writer = request_body.writer;
        tokio::spawn(async move {
            if !bytes.is_empty() {
                let _ = writer.write(&bytes).await;
            }
            let _ = writer.close().await;
        });

        if !request.hand_over(response.reader) {
            return Err(internal_error(
                "the server stopped waiting for the response",
            ));
        }
        Ok((request_body.reader_ref, response.writer_ref))
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
        let in_flight = request_id(&call).and_then(|id| self.requests.get(id));
        let public_webhooks_only = in_flight.as_ref().is_some_and(|r| r.public_webhooks_only());
        let field = |name: &str| call.input.get(name).and_then(Value::as_str).unwrap_or("");
        let (route, path_params) = match self
            .match_route(field("method"), field("path"), public_webhooks_only)
            .await
        {
            Ok(matched) => matched,
            Err(response) => return Ok(response),
        };
        // A route is filed before its observer is accepted.
        let Some(obs) = observers.into_iter().find(|obs| obs.id() == route.id()) else {
            return Ok(not_found());
        };
        let Some(in_flight) = in_flight else {
            return Err(Error::Handler(format!(
                "{ON_REQUEST_FN}: no request in flight for this call"
            )));
        };
        in_flight.set_route(&route.http_path);
        if in_flight.body().is_none() {
            return Ok(payload_too_large());
        }
        if !obs.matches(&call) {
            tracing::debug!(
                observer_id = %obs.id(),
                target = %obs.target(),
                "Observer filters rejected the request, skipping handler"
            );
            return Ok(filters_not_met());
        }
        let request: InboundRequest =
            serde_json::from_value(call.input).map_err(|e| Error::Serde(e.to_string()))?;

        // Global middleware (config-driven, sorted by priority at load time),
        // then the route's own, in list order.
        let (timeout_ms, global) = {
            let config = self.config.read().await;
            let global: Vec<String> = config
                .middleware
                .iter()
                .filter(|mw| mw.phase == "preHandler")
                .map(|mw| mw.function_id.clone())
                .collect();
            (config.default_timeout, global)
        };
        let middleware_input = middleware::build_middleware_input(
            &path_params,
            &request.query_params,
            &request.headers,
            &request.method,
        );
        for mw_fn_id in global.iter().chain(&route.middleware_function_ids) {
            match middleware::execute_middleware(
                &self.iii,
                mw_fn_id,
                middleware_input.clone(),
                timeout_ms,
            )
            .await
            {
                Ok(MiddlewareOutcome::Continue) => {}
                Err(response) => return Ok(response),
            }
        }

        let (request_body, response) = match self.open_channels(&in_flight).await {
            Ok(refs) => refs,
            Err(response) => return Ok(response),
        };
        let target_request = HttpRequest {
            query_params: request.query_params,
            path_params,
            headers: request.headers,
            path: route.http_path,
            method: request.method,
            body: request.body,
            request_body,
            response,
        };
        let payload =
            serde_json::to_value(&target_request).map_err(|e| Error::Serde(e.to_string()))?;
        self.iii
            .call(CallRequest::observer(&obs, payload).timeout(Duration::from_millis(timeout_ms)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_format_is_the_request_as_received() {
        let schema = request_format();
        for field in ["method", "path", "query_params", "headers", "body"] {
            assert!(
                schema["properties"].get(field).is_some(),
                "request_format lacks {field}"
            );
        }
        for field in ["path_params", "request_body", "response"] {
            assert!(
                schema["properties"].get(field).is_none(),
                "request_format has {field}"
            );
        }
    }

    #[test]
    fn the_request_in_flight_is_read_from_the_call_metadata() {
        let call = Call {
            metadata: Some(json!({ REQUEST_ID_KEY: "req-1" })),
            ..Call::default()
        };
        assert_eq!(request_id(&call), Some("req-1"));
        assert_eq!(request_id(&Call::default()), None);
        let other = Call {
            metadata: Some(json!({ REQUEST_ID_KEY: 7 })),
            ..Call::default()
        };
        assert_eq!(request_id(&other), None);
    }

    #[test]
    fn hook_answers_are_responses() {
        use crate::types::HttpResponse;
        let missing = HttpResponse::from_function_return(not_found());
        assert_eq!(missing.status_code, 404);
        assert_eq!(missing.body["error"]["code"], "NOT_FOUND");
        assert!(missing.header("allow").is_none());
        let wrong_method =
            HttpResponse::from_function_return(method_not_allowed(&["GET".into(), "POST".into()]));
        assert_eq!(wrong_method.status_code, 405);
        assert_eq!(wrong_method.header("allow"), Some("GET, POST"));
        assert_eq!(wrong_method.body["error"]["code"], "METHOD_NOT_ALLOWED");
        let too_large = HttpResponse::from_function_return(payload_too_large());
        assert_eq!(too_large.status_code, 413);
        assert_eq!(too_large.body["error"]["code"], "PAYLOAD_TOO_LARGE");
        let rejected = HttpResponse::from_function_return(filters_not_met());
        assert_eq!(rejected.status_code, 422);
        assert_eq!(rejected.body["error"]["code"], "CONDITION_NOT_MET");
        assert_eq!(rejected.body["skipped"], true);
    }
}
