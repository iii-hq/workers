//! Route table: registers `http` trigger instances as routes and matches
//! incoming requests (method + path) against them, extracting path params.
//!
//! Ported from the in-engine `iii-http` worker's `RestApiWorker`
//! (`route_signature` / `register_router` conflict check in `api_core.rs`,
//! `extract_path_params` in `views.rs`), adapted to a standalone, axum-free
//! `RouteTable` keyed by trigger id.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use colored::Colorize;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use tokio::sync::RwLock;

use crate::types::HttpTriggerConfig;

/// A single registered HTTP route, derived from an `http` trigger instance.
#[derive(Debug, Clone)]
pub struct Route {
    pub trigger_id: String,
    pub function_id: String,
    pub http_path: String,
    pub http_method: String,
    pub condition_function_id: Option<String>,
    pub middleware_function_ids: Vec<String>,
}

/// Computes a route's "shape" signature: method + path with every `:param`
/// segment collapsed to a positional placeholder. Two routes with the same
/// signature collide regardless of their param names (e.g. `/a/:x/:y` and
/// `/a/:y/:x`).
pub fn route_signature(http_method: &str, http_path: &str) -> String {
    let shape = http_path
        .split('/')
        .map(|segment| {
            if segment.starts_with(':') {
                "{}"
            } else {
                segment
            }
        })
        .collect::<Vec<&str>>()
        .join("/");
    format!("{}:{}", http_method.to_uppercase(), shape)
}

/// Counts the `:param` segments in a registered path. Used to rank matching
/// routes so literal segments win over parameters.
fn param_segment_count(registered_path: &str) -> usize {
    registered_path
        .split('/')
        .filter(|s| s.starts_with(':'))
        .count()
}

/// Matches `actual_path` against `registered_path` (which may contain
/// `:param` segments), returning extracted path params. Returns `None` if
/// the segment counts differ or any literal segment doesn't match exactly.
pub fn extract_path_params(
    registered_path: &str,
    actual_path: &str,
) -> Option<HashMap<String, String>> {
    let registered_segments: Vec<&str> = registered_path
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let actual_segments: Vec<&str> = actual_path.split('/').filter(|s| !s.is_empty()).collect();

    if registered_segments.len() != actual_segments.len() {
        return None;
    }

    let mut params = HashMap::new();
    for (registered_seg, actual_seg) in registered_segments.iter().zip(actual_segments.iter()) {
        if let Some(param_name) = registered_seg.strip_prefix(':') {
            params.insert(param_name.to_string(), actual_seg.to_string());
        } else if registered_seg != actual_seg {
            return None;
        }
    }

    Some(params)
}

/// Registry of routes for the `http` trigger type, keyed by trigger id so a
/// trigger can be unregistered by id alone. Rejects inserting a route whose
/// method+path shape conflicts with another, differently-id'd route already
/// registered (same shape, different param names) -- matching it would be
/// ambiguous. Re-inserting under the same trigger id replaces the entry.
#[derive(Default)]
pub struct RouteTable {
    by_id: HashMap<String, Route>,
}

impl RouteTable {
    /// Inserts or replaces a route. Fails if another trigger already owns a
    /// route with the same method+path signature.
    pub fn insert(&mut self, route: Route) -> Result<(), String> {
        let signature = route_signature(&route.http_method, &route.http_path);
        let conflict = self.by_id.values().find(|existing| {
            existing.trigger_id != route.trigger_id
                && route_signature(&existing.http_method, &existing.http_path) == signature
        });
        if let Some(existing) = conflict {
            return Err(format!(
                "Route '{} {}' conflicts with already-registered route '{} {}': \
                 routes with identical structure but different path-parameter \
                 names are not supported",
                route.http_method.to_uppercase(),
                route.http_path,
                existing.http_method.to_uppercase(),
                existing.http_path
            ));
        }

        self.by_id.insert(route.trigger_id.clone(), route);
        Ok(())
    }

    /// Removes and returns the route registered under `id`, if any.
    pub fn remove_by_trigger_id(&mut self, id: &str) -> Option<Route> {
        self.by_id.remove(id)
    }

    /// Finds the registered route whose method matches (case insensitive) and
    /// whose path matches `actual_path`, returning a clone of the route plus
    /// its extracted path params.
    ///
    /// When several routes match (e.g. `/items/:id` and `/items/copy` both
    /// match `/items/copy`), the more literal route wins: candidates are
    /// scored by their number of `:param` segments and the lowest score is
    /// chosen. Iteration order over the backing `HashMap` is nondeterministic,
    /// so this scoring is what makes the result stable. The engine gets the
    /// same precedence for free from axum's matcher.
    pub fn match_route(
        &self,
        method: &str,
        actual_path: &str,
    ) -> Option<(Route, HashMap<String, String>)> {
        let method = method.to_uppercase();
        self.by_id
            .values()
            .filter(|route| route.http_method.to_uppercase() == method)
            .filter_map(|route| {
                extract_path_params(&route.http_path, actual_path)
                    .map(|params| (param_segment_count(&route.http_path), route.clone(), params))
            })
            .min_by_key(|(score, _, _)| *score)
            .map(|(_, route, params)| (route, params))
    }

    /// Methods registered for any route whose path pattern matches
    /// `actual_path` (ignoring HTTP method). Used to distinguish 404 (no such
    /// path) from 405 (path exists, method not allowed). Returns uppercased
    /// methods, deduped, in sorted order.
    pub fn allowed_methods(&self, actual_path: &str) -> Vec<String> {
        let mut methods: Vec<String> = self
            .by_id
            .values()
            .filter(|route| extract_path_params(&route.http_path, actual_path).is_some())
            .map(|route| route.http_method.to_uppercase())
            .collect();
        methods.sort();
        methods.dedup();
        methods
    }
}

/// Bridges the SDK's `TriggerHandler` callbacks to the [`RouteTable`]: turns
/// `register_trigger`/`unregister_trigger` calls (issued by the engine when
/// functions bind `http` triggers) into route inserts/removals.
///
/// Cheap to clone -- the table is shared via `Arc<RwLock<_>>` so the same
/// handle can be handed to the server task that reads routes to match
/// incoming requests.
#[derive(Clone, Default)]
pub struct HttpTriggerHandler {
    pub routes: Arc<RwLock<RouteTable>>,
}

impl HttpTriggerHandler {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh, empty, shareable route table -- handed to both this handler
    /// and the server task so they observe the same registrations.
    pub fn shared_routes() -> Arc<RwLock<RouteTable>> {
        Arc::new(RwLock::new(RouteTable::default()))
    }
}

#[async_trait]
impl TriggerHandler for HttpTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let tc: HttpTriggerConfig = serde_json::from_value(config.config.clone())
            .map_err(|e| Error::Handler(format!("invalid http trigger config: {e}")))?;
        let route = Route {
            trigger_id: config.id.clone(),
            function_id: config.function_id.clone(),
            http_path: tc.api_path,
            http_method: tc.http_method.to_uppercase(),
            condition_function_id: tc.condition_function_id,
            middleware_function_ids: tc.middleware_function_ids,
        };
        self.routes
            .write()
            .await
            .insert(route.clone())
            .map_err(Error::Handler)?;
        // Same registration line as the engine's builtin iii-http (api_core.rs).
        tracing::info!(
            "{} Endpoint {} → {}",
            "[REGISTERED]".green(),
            format!("{} {}", route.http_method, route.http_path)
                .bright_yellow()
                .bold(),
            route.function_id.purple()
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        // The SDK only populates `id` for unregister -- function_id/config
        // are stubs -- so the table must be (and is) keyed by trigger id.
        if let Some(route) = self.routes.write().await.remove_by_trigger_id(&config.id) {
            tracing::info!(
                "{} Endpoint {} → {}",
                "[UNREGISTERED]".yellow(),
                format!("{} {}", route.http_method, route.http_path)
                    .bright_yellow()
                    .bold(),
                route.function_id.purple()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(trigger_id: &str, method: &str, path: &str) -> Route {
        Route {
            trigger_id: trigger_id.to_string(),
            function_id: format!("fn-{trigger_id}"),
            http_path: path.to_string(),
            http_method: method.to_string(),
            condition_function_id: None,
            middleware_function_ids: Vec::new(),
        }
    }

    // =========================================================================
    // route_signature
    // =========================================================================

    #[test]
    fn route_signature_collapses_param_names() {
        let a = route_signature("GET", "/a/:x/:y");
        let b = route_signature("GET", "/a/:y/:x");
        assert_eq!(a, b);
    }

    #[test]
    fn route_signature_distinguishes_method() {
        let get = route_signature("GET", "/items/:id");
        let post = route_signature("POST", "/items/:id");
        assert_ne!(get, post);
    }

    #[test]
    fn route_signature_distinguishes_literal_segments() {
        let a = route_signature("GET", "/items/:id/calls");
        let b = route_signature("GET", "/items/:id/messages");
        assert_ne!(a, b);
    }

    // =========================================================================
    // extract_path_params
    // =========================================================================

    #[test]
    fn extract_path_params_matches_single_param() {
        let params = extract_path_params("/users/:id", "/users/42").unwrap();
        assert_eq!(params.get("id"), Some(&"42".to_string()));
    }

    #[test]
    fn extract_path_params_rejects_different_segment_count() {
        assert!(extract_path_params("/users/:id", "/users/42/extra").is_none());
    }

    #[test]
    fn extract_path_params_rejects_literal_mismatch() {
        assert!(extract_path_params("/users/:id/calls", "/users/42/messages").is_none());
    }

    #[test]
    fn extract_path_params_matches_no_params() {
        let params = extract_path_params("/health", "/health").unwrap();
        assert!(params.is_empty());
    }

    // =========================================================================
    // RouteTable
    // =========================================================================

    #[test]
    fn match_route_extracts_path_params() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();

        let (matched, params) = table.match_route("GET", "/users/42").unwrap();
        assert_eq!(matched.trigger_id, "t1");
        assert_eq!(params.get("id"), Some(&"42".to_string()));
    }

    #[test]
    fn match_route_prefers_literal_over_param() {
        // `/items/copy` and `/items/:id` both match `/items/copy`; the literal
        // route must win regardless of HashMap iteration order.
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/items/:id")).unwrap();
        table.insert(route("t2", "GET", "/items/copy")).unwrap();

        let (matched, params) = table.match_route("GET", "/items/copy").unwrap();
        assert_eq!(matched.trigger_id, "t2");
        assert!(params.is_empty());

        // A non-literal path still falls through to the param route.
        let (matched, params) = table.match_route("GET", "/items/42").unwrap();
        assert_eq!(matched.trigger_id, "t1");
        assert_eq!(params.get("id"), Some(&"42".to_string()));
    }

    #[test]
    fn match_route_returns_none_for_wrong_method() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();

        assert!(table.match_route("POST", "/users/42").is_none());
    }

    #[test]
    fn match_route_returns_none_for_nonexistent_path() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();

        assert!(table.match_route("GET", "/orders/42").is_none());
    }

    #[test]
    fn insert_rejects_conflicting_signature_from_different_trigger() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/a/:x")).unwrap();

        let result = table.insert(route("t2", "GET", "/a/:y"));
        assert!(result.is_err());
    }

    #[test]
    fn insert_allows_same_trigger_id_reinsert_as_replace() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/a/:x")).unwrap();

        let result = table.insert(route("t1", "GET", "/a/:x/:y"));
        assert!(result.is_ok());

        let (matched, _) = table.match_route("GET", "/a/1/2").unwrap();
        assert_eq!(matched.trigger_id, "t1");
    }

    #[test]
    fn allowed_methods_lists_all_methods_for_matching_path() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();
        table.insert(route("t2", "POST", "/users/:id")).unwrap();

        assert_eq!(table.allowed_methods("/users/42"), vec!["GET", "POST"]);
    }

    #[test]
    fn allowed_methods_empty_for_nonmatching_path() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();

        assert!(table.allowed_methods("/orders/42").is_empty());
    }

    #[test]
    fn remove_by_trigger_id_removes_then_reports_false() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();

        assert!(table.remove_by_trigger_id("t1").is_some());
        assert!(table.remove_by_trigger_id("t1").is_none());
        assert!(table.match_route("GET", "/users/42").is_none());
    }

    // =========================================================================
    // HttpTriggerHandler
    // =========================================================================

    fn trigger_config(id: &str, function_id: &str, config: serde_json::Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn register_trigger_inserts_matchable_route() {
        let handler = HttpTriggerHandler::new();
        handler
            .register_trigger(trigger_config(
                "t1",
                "fn.a",
                serde_json::json!({"api_path": "/a", "http_method": "POST"}),
            ))
            .await
            .unwrap();

        let routes = handler.routes.read().await;
        let (matched, _) = routes.match_route("POST", "/a").unwrap();
        assert_eq!(matched.function_id, "fn.a");
        assert_eq!(matched.trigger_id, "t1");
    }

    #[tokio::test]
    async fn register_trigger_rejects_invalid_config() {
        let handler = HttpTriggerHandler::new();
        let result = handler
            .register_trigger(trigger_config("t1", "fn.a", serde_json::json!({})))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unregister_trigger_removes_route_using_only_id_stub() {
        let handler = HttpTriggerHandler::new();
        handler
            .register_trigger(trigger_config(
                "t1",
                "fn.a",
                serde_json::json!({"api_path": "/a", "http_method": "post"}),
            ))
            .await
            .unwrap();

        // The SDK passes a stub on unregister: only `id` is populated.
        handler
            .unregister_trigger(trigger_config("t1", "", serde_json::Value::Null))
            .await
            .unwrap();

        let routes = handler.routes.read().await;
        assert!(routes.match_route("POST", "/a").is_none());
    }
}
