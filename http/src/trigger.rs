//! Route table: registers `http` trigger instances as routes and matches
//! incoming requests (method + path) against them, extracting path params.
//!
//! Ported from the in-engine `iii-http` worker's `RestApiWorker`
//! (`route_signature` / `register_router` conflict check in `api_core.rs`,
//! `extract_path_params` in `views.rs`), adapted to a standalone, axum-free
//! `RouteTable` keyed by trigger id.

use std::collections::HashMap;

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

    /// Removes the route registered under `id`, if any. Returns whether a
    /// route was actually removed.
    pub fn remove_by_trigger_id(&mut self, id: &str) -> bool {
        self.by_id.remove(id).is_some()
    }

    /// Finds the first registered route whose method matches (case
    /// insensitive) and whose path matches `actual_path`, returning a clone
    /// of the route plus its extracted path params.
    pub fn match_route(
        &self,
        method: &str,
        actual_path: &str,
    ) -> Option<(Route, HashMap<String, String>)> {
        let method = method.to_uppercase();
        self.by_id.values().find_map(|route| {
            if route.http_method.to_uppercase() != method {
                return None;
            }
            extract_path_params(&route.http_path, actual_path).map(|params| (route.clone(), params))
        })
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
    fn remove_by_trigger_id_removes_then_reports_false() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();

        assert!(table.remove_by_trigger_id("t1"));
        assert!(!table.remove_by_trigger_id("t1"));
        assert!(table.match_route("GET", "/users/42").is_none());
    }
}
