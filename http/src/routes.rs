//! Route table: one route per `http::on-request` observer, matched against
//! incoming requests (method + path) with path params extracted.
//!
//! Ported from the in-engine `iii-http` worker's `RestApiWorker`
//! (`route_signature` / `register_router` conflict check in `api_core.rs`,
//! `extract_path_params` in `views.rs`), adapted to a standalone, axum-free
//! `RouteTable` keyed by observer id.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use colored::Colorize;

/// A single registered HTTP route, declared by the args of one
/// `http::on-request` observer.
#[derive(Debug, Clone)]
pub struct Route {
    /// The observer that declared the route. The server names it on each call
    /// of `http::on-request`, whose deliver hook calls through it.
    pub observer_id: String,
    pub http_path: String,
    pub http_method: String,
    pub public_webhook: bool,
    /// Served only on a connection accepted on a loopback local address
    /// (see [`Admission`]).
    pub loopback_only: bool,
    pub middleware_function_ids: Vec<String>,
}

impl Route {
    /// The route's id: its observer's id.
    pub fn id(&self) -> &str {
        &self.observer_id
    }
}

/// Same registration lines as the engine's builtin iii-http (api_core.rs).
/// `target` is the observer's target, read from the observer at hand.
pub(crate) fn log_registered(route: &Route, target: &str) {
    tracing::info!(
        "{} Endpoint {} → {}",
        "[REGISTERED]".green(),
        format!("{} {}", route.http_method, route.http_path)
            .bright_yellow()
            .bold(),
        target.purple()
    );
}

pub(crate) fn log_unregistered(route: &Route, target: &str) {
    tracing::info!(
        "{} Endpoint {} → {}",
        "[UNREGISTERED]".yellow(),
        format!("{} {}", route.http_method, route.http_path)
            .bright_yellow()
            .bold(),
        target.purple()
    );
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

fn listener_path_params(
    registered: &str,
    actual: &str,
    public_only: bool,
) -> Option<HashMap<String, String>> {
    if !public_only {
        return extract_path_params(registered, actual);
    }
    let registered: Vec<_> = registered.split('/').collect();
    let actual: Vec<_> = actual.split('/').collect();
    if registered.len() != actual.len() {
        return None;
    }
    let mut params = HashMap::new();
    for (expected, received) in registered.iter().zip(actual) {
        if let Some(name) = expected.strip_prefix(':') {
            if received.is_empty() {
                return None;
            }
            params.insert(name.to_string(), received.to_string());
        } else if *expected != received {
            return None;
        }
    }
    Some(params)
}

/// Whether `ip` is a loopback address: `127.0.0.0/8`, `::1`, or an
/// IPv4-mapped loopback address (`::ffff:127.x.y.z`).
pub fn is_loopback_ip(ip: IpAddr) -> bool {
    // TDD stub: replaced by the real check.
    let _ = ip;
    false
}

/// What one accepted connection may reach, decided per connection and read
/// on every request: the restricted listener admits only `public_webhook`
/// routes, and a `loopback_only` route is admitted only on a connection whose
/// LOCAL address (the socket's own address, not the peer's) is loopback.
///
/// A refused route behaves as if it did not exist on that connection: it
/// neither matches nor contributes to the `Allow` header of a 405.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admission {
    /// The connection was accepted by the restricted webhook listener.
    pub public_webhooks_only: bool,
    /// The connection's local address is loopback.
    pub loopback_connection: bool,
}

impl Admission {
    pub const fn new(public_webhooks_only: bool, loopback_connection: bool) -> Self {
        Self {
            public_webhooks_only,
            loopback_connection,
        }
    }

    /// The admission of a connection accepted on `local_addr` by the normal
    /// (`public_webhooks_only == false`) or the restricted listener. An
    /// unknown local address is not loopback.
    pub fn for_connection(public_webhooks_only: bool, local_addr: Option<SocketAddr>) -> Self {
        Self::new(
            public_webhooks_only,
            local_addr.is_some_and(|addr| is_loopback_ip(addr.ip())),
        )
    }

    /// Whether `route` exists for this connection.
    pub fn admits(&self, route: &Route) -> bool {
        // TDD stub: ignores loopback_only.
        !self.public_webhooks_only || route.public_webhook
    }
}

/// Registry of routes, keyed by observer id so a route can be removed by id
/// alone. Rejects inserting a route whose method+path shape conflicts with
/// another observer's route already registered (same shape, different param
/// names): matching it would be ambiguous. The later observer is rejected
/// with `HTTP_ROUTE_CONFLICT`. Re-inserting under the same id replaces the
/// entry.
#[derive(Default)]
pub struct RouteTable {
    by_id: HashMap<String, Route>,
}

impl RouteTable {
    /// Inserts or replaces a route. Fails if another observer already owns a
    /// route with the same method+path signature.
    pub fn insert(&mut self, route: Route) -> Result<(), String> {
        let signature = route_signature(&route.http_method, &route.http_path);
        let conflict = self.by_id.values().find(|existing| {
            existing.observer_id != route.observer_id
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

        self.by_id.insert(route.observer_id.clone(), route);
        Ok(())
    }

    /// Removes and returns the route the observer `id` declared, if any.
    pub fn remove(&mut self, id: &str) -> Option<Route> {
        self.by_id.remove(id)
    }

    /// The route the observer `id` declared, if any.
    pub fn get(&self, id: &str) -> Option<&Route> {
        self.by_id.get(id)
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
        self.match_route_for_listener(method, actual_path, Admission::new(false, true))
    }

    /// Same registry and precedence, filtered by the connection's
    /// [`Admission`] before matching: a route it refuses does not exist for
    /// the request. On the restricted listener, no decoding, slash
    /// normalization, or method aliasing.
    pub fn match_route_for_listener(
        &self,
        method: &str,
        actual_path: &str,
        admission: Admission,
    ) -> Option<(Route, HashMap<String, String>)> {
        let public_only = admission.public_webhooks_only;
        let method = if public_only {
            method.to_string()
        } else {
            method.to_uppercase()
        };
        self.by_id
            .values()
            .filter(|route| admission.admits(route) && route.http_method.to_uppercase() == method)
            .filter_map(|route| {
                listener_path_params(&route.http_path, actual_path, public_only)
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
        self.allowed_methods_for_listener(actual_path, Admission::new(false, true))
    }

    /// Routes the connection's [`Admission`] refuses must not leak through
    /// the Allow header: private routes on the restricted listener,
    /// `loopback_only` routes on a non-loopback connection.
    pub fn allowed_methods_for_listener(
        &self,
        actual_path: &str,
        admission: Admission,
    ) -> Vec<String> {
        let public_only = admission.public_webhooks_only;
        let mut methods: Vec<String> = self
            .by_id
            .values()
            .filter(|route| admission.admits(route))
            .filter(|route| {
                listener_path_params(&route.http_path, actual_path, public_only).is_some()
            })
            .map(|route| route.http_method.to_uppercase())
            .collect();
        methods.sort();
        methods.dedup();
        methods
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(observer_id: &str, method: &str, path: &str) -> Route {
        Route {
            observer_id: observer_id.to_string(),
            http_path: path.to_string(),
            http_method: method.to_string(),
            public_webhook: false,
            loopback_only: false,
            middleware_function_ids: Vec::new(),
        }
    }

    /// The restricted listener, on a non-loopback connection.
    const WEBHOOK: Admission = Admission::new(true, false);

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
    fn restricted_matching_filters_routes_methods_and_path_variants() {
        let mut table = RouteTable::default();
        table.insert(route("private", "GET", "/private")).unwrap();
        table.insert(route("delete", "DELETE", "/hook")).unwrap();
        let mut public = route("public", "POST", "/hook");
        public.public_webhook = true;
        table.insert(public).unwrap();
        for path in [
            "/private",
            "/private/",
            "//private",
            "/%70rivate",
            "/private%2f",
        ] {
            assert!(
                table
                    .match_route_for_listener("GET", path, WEBHOOK)
                    .is_none()
            );
            assert!(table.allowed_methods_for_listener(path, WEBHOOK).is_empty());
        }
        assert!(
            table
                .match_route_for_listener("POST", "/hook", WEBHOOK)
                .is_some()
        );
        assert_eq!(
            table.allowed_methods_for_listener("/hook", WEBHOOK),
            vec!["POST"]
        );
        for method in ["GET", "HEAD", "DELETE", "OPTIONS", "post"] {
            assert!(
                table
                    .match_route_for_listener(method, "/hook", WEBHOOK)
                    .is_none()
            );
        }
        for path in ["/hook/", "//hook", "/%68ook", "/hook%2f"] {
            assert!(
                table
                    .match_route_for_listener("POST", path, WEBHOOK)
                    .is_none()
            );
        }
        // Existing normal-listener normalization stays backwards compatible.
        assert!(table.match_route("get", "/private/").is_some());
        let mut revoked = route("public", "POST", "/hook");
        revoked.public_webhook = false;
        table.insert(revoked).unwrap();
        assert!(
            table
                .match_route_for_listener("POST", "/hook", WEBHOOK)
                .is_none()
        );
    }

    #[test]
    fn restricted_parameter_route_never_selects_private_handler() {
        let mut table = RouteTable::default();
        let mut public = route("public", "POST", "/hooks/:id");
        public.public_webhook = true;
        table.insert(public).unwrap();
        table
            .insert(route("private", "POST", "/hooks/admin"))
            .unwrap();
        let (matched, params) = table
            .match_route_for_listener("POST", "/hooks/admin", WEBHOOK)
            .unwrap();
        assert_eq!(matched.id(), "public");
        assert_eq!(params["id"], "admin");
        assert_eq!(
            table.match_route("POST", "/hooks/admin").unwrap().0.id(),
            "private"
        );
        assert!(
            table
                .match_route_for_listener("POST", "/hooks/", WEBHOOK)
                .is_none()
        );
    }

    #[test]
    fn match_route_extracts_path_params() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();

        let (matched, params) = table.match_route("GET", "/users/42").unwrap();
        assert_eq!(matched.id(), "t1");
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
        assert_eq!(matched.id(), "t2");
        assert!(params.is_empty());

        // A non-literal path still falls through to the param route.
        let (matched, params) = table.match_route("GET", "/items/42").unwrap();
        assert_eq!(matched.id(), "t1");
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
    fn insert_rejects_conflicting_signature_from_different_observer() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/a/:x")).unwrap();

        let result = table.insert(route("t2", "GET", "/a/:y"));
        assert!(result.is_err());
    }

    #[test]
    fn insert_allows_same_observer_id_reinsert_as_replace() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/a/:x")).unwrap();

        let result = table.insert(route("t1", "GET", "/a/:x/:y"));
        assert!(result.is_ok());

        let (matched, _) = table.match_route("GET", "/a/1/2").unwrap();
        assert_eq!(matched.id(), "t1");
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
    fn remove_removes_then_reports_none() {
        let mut table = RouteTable::default();
        table.insert(route("t1", "GET", "/users/:id")).unwrap();

        assert_eq!(
            table.get("t1").map(|r| r.http_path.as_str()),
            Some("/users/:id")
        );
        assert!(table.remove("t1").is_some());
        assert!(table.remove("t1").is_none());
        assert!(table.get("t1").is_none());
        assert!(table.match_route("GET", "/users/42").is_none());
    }

    #[test]
    fn brace_and_colon_routes_share_a_shape() {
        let mut table = RouteTable::default();
        table
            .insert(route(
                "t1",
                "GET",
                &crate::types::normalize_api_path("/tasks/{tid}"),
            ))
            .unwrap();
        let (matched, params) = table.match_route("GET", "/tasks/123").unwrap();
        assert_eq!(matched.http_path, "/tasks/:tid");
        assert_eq!(params.get("tid"), Some(&"123".to_string()));
        assert!(
            table
                .insert(route(
                    "t2",
                    "GET",
                    &crate::types::normalize_api_path("/tasks/{other}")
                ))
                .is_err()
        );
    }

    // =========================================================================
    // loopback_only admission
    // =========================================================================

    /// The normal listener, on a connection accepted on a loopback address.
    const LOOPBACK: Admission = Admission::new(false, true);
    /// The normal listener, on a connection accepted on any other address.
    const REMOTE: Admission = Admission::new(false, false);

    fn loopback_route(observer_id: &str, method: &str, path: &str) -> Route {
        Route {
            loopback_only: true,
            ..route(observer_id, method, path)
        }
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_addresses_are_loopback() {
        for ip in [
            "127.0.0.1",
            "127.0.0.2",
            "127.255.255.254",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:127.10.20.30",
        ] {
            assert!(is_loopback_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn other_addresses_are_not_loopback() {
        for ip in [
            "0.0.0.0",
            "10.0.0.1",
            "192.168.1.20",
            "172.16.0.1",
            "128.0.0.1",
            "126.255.255.255",
            "8.8.8.8",
            "::",
            "::2",
            "fe80::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
            "::ffff:0.0.0.0",
            // IPv4-compatible (deprecated), not IPv4-mapped: not loopback.
            "::127.0.0.1",
        ] {
            assert!(!is_loopback_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn admission_reads_the_connections_local_address() {
        for (local, loopback) in [
            ("127.0.0.1:3111", true),
            ("127.0.0.53:80", true),
            ("[::1]:3111", true),
            ("[::ffff:127.0.0.1]:3111", true),
            ("10.1.2.3:3111", false),
            ("192.168.0.10:3111", false),
            ("[2001:db8::7]:3111", false),
            ("[::ffff:192.168.0.10]:3111", false),
        ] {
            for public_webhooks_only in [false, true] {
                assert_eq!(
                    Admission::for_connection(public_webhooks_only, Some(addr(local))),
                    Admission::new(public_webhooks_only, loopback),
                    "{local}"
                );
            }
        }
        // An unknown local address is not loopback: fail closed.
        assert_eq!(
            Admission::for_connection(false, None),
            Admission::new(false, false)
        );
    }

    #[test]
    fn admission_of_a_route() {
        let plain = route("plain", "GET", "/a");
        let local = loopback_route("local", "GET", "/b");
        let public = Route {
            public_webhook: true,
            ..route("public", "POST", "/c")
        };
        assert!(LOOPBACK.admits(&plain));
        assert!(REMOTE.admits(&plain));
        assert!(LOOPBACK.admits(&local));
        assert!(!REMOTE.admits(&local));
        assert!(LOOPBACK.admits(&public));
        assert!(REMOTE.admits(&public));
        // The restricted listener admits only public_webhook routes, whatever
        // the connection.
        for loopback in [false, true] {
            let webhook = Admission::new(true, loopback);
            assert!(!webhook.admits(&plain));
            assert!(!webhook.admits(&local));
            assert!(webhook.admits(&public));
        }
    }

    #[test]
    fn loopback_only_route_is_served_on_a_loopback_connection() {
        let mut table = RouteTable::default();
        table.insert(loopback_route("mcp", "POST", "/mcp")).unwrap();
        let (matched, params) = table
            .match_route_for_listener("POST", "/mcp", LOOPBACK)
            .unwrap();
        assert_eq!(matched.id(), "mcp");
        assert!(matched.loopback_only);
        assert!(params.is_empty());
        // Wrong method on a loopback connection: the route exists, so 405.
        assert!(
            table
                .match_route_for_listener("GET", "/mcp", LOOPBACK)
                .is_none()
        );
        assert_eq!(
            table.allowed_methods_for_listener("/mcp", LOOPBACK),
            vec!["POST"]
        );
    }

    #[test]
    fn loopback_only_route_does_not_exist_on_a_remote_connection() {
        let mut table = RouteTable::default();
        table.insert(loopback_route("mcp", "POST", "/mcp")).unwrap();
        for method in ["POST", "post", "GET", "DELETE"] {
            assert!(
                table
                    .match_route_for_listener(method, "/mcp", REMOTE)
                    .is_none(),
                "{method}"
            );
        }
        // No Allow leak: the only route of the path is refused, so it is a
        // 404 (empty), not a 405.
        assert!(
            table
                .allowed_methods_for_listener("/mcp", REMOTE)
                .is_empty()
        );
        // Nor on the restricted listener, over loopback or not.
        for loopback in [false, true] {
            let webhook = Admission::new(true, loopback);
            assert!(
                table
                    .match_route_for_listener("POST", "/mcp", webhook)
                    .is_none()
            );
            assert!(
                table
                    .allowed_methods_for_listener("/mcp", webhook)
                    .is_empty()
            );
        }
    }

    #[test]
    fn loopback_only_parameter_route_is_refused_on_a_remote_connection() {
        let mut table = RouteTable::default();
        table
            .insert(loopback_route("admin", "GET", "/admin/:section"))
            .unwrap();
        let (_, params) = table
            .match_route_for_listener("GET", "/admin/users", LOOPBACK)
            .unwrap();
        assert_eq!(params["section"], "users");
        assert!(
            table
                .match_route_for_listener("GET", "/admin/users", REMOTE)
                .is_none()
        );
        assert!(
            table
                .allowed_methods_for_listener("/admin/users", REMOTE)
                .is_empty()
        );
    }

    #[test]
    fn other_routes_are_unaffected_by_the_connection() {
        let mut table = RouteTable::default();
        table.insert(route("users", "GET", "/users/:id")).unwrap();
        table.insert(loopback_route("mcp", "POST", "/mcp")).unwrap();
        for admission in [LOOPBACK, REMOTE] {
            let (matched, params) = table
                .match_route_for_listener("GET", "/users/42", admission)
                .unwrap();
            assert_eq!(matched.id(), "users");
            assert_eq!(params["id"], "42");
            assert_eq!(
                table.allowed_methods_for_listener("/users/42", admission),
                vec!["GET"]
            );
            assert!(
                table
                    .match_route_for_listener("GET", "/nothing", admission)
                    .is_none()
            );
        }
    }

    #[test]
    fn refused_methods_do_not_leak_into_allow() {
        // Same path: GET is loopback_only, POST is not.
        let mut table = RouteTable::default();
        table
            .insert(loopback_route("read", "GET", "/state"))
            .unwrap();
        table.insert(route("write", "POST", "/state")).unwrap();
        assert_eq!(
            table.allowed_methods_for_listener("/state", LOOPBACK),
            vec!["GET", "POST"]
        );
        assert_eq!(
            table.allowed_methods_for_listener("/state", REMOTE),
            vec!["POST"]
        );
        assert!(
            table
                .match_route_for_listener("GET", "/state", REMOTE)
                .is_none()
        );
        assert_eq!(
            table
                .match_route_for_listener("POST", "/state", REMOTE)
                .unwrap()
                .0
                .id(),
            "write"
        );
    }

    #[test]
    fn a_refused_literal_route_falls_through_as_if_absent() {
        // `/items/copy` (loopback_only) beats `/items/:id` over loopback; on a
        // remote connection it does not exist, so `/items/:id` serves.
        let mut table = RouteTable::default();
        table.insert(route("item", "GET", "/items/:id")).unwrap();
        table
            .insert(loopback_route("copy", "GET", "/items/copy"))
            .unwrap();
        assert_eq!(
            table
                .match_route_for_listener("GET", "/items/copy", LOOPBACK)
                .unwrap()
                .0
                .id(),
            "copy"
        );
        let (matched, params) = table
            .match_route_for_listener("GET", "/items/copy", REMOTE)
            .unwrap();
        assert_eq!(matched.id(), "item");
        assert_eq!(params["id"], "copy");
    }

    #[test]
    fn loopback_only_shares_the_conflict_namespace() {
        let mut table = RouteTable::default();
        table.insert(loopback_route("a", "GET", "/a/:x")).unwrap();
        // Same shape, plain route: still HTTP_ROUTE_CONFLICT territory.
        assert!(table.insert(route("b", "GET", "/a/:y")).is_err());
        assert!(table.insert(loopback_route("c", "GET", "/a/:z")).is_err());
        // And the other way round.
        let mut table = RouteTable::default();
        table.insert(route("a", "GET", "/b/:x")).unwrap();
        let err = table
            .insert(loopback_route("b", "GET", "/b/:y"))
            .unwrap_err();
        assert!(err.contains("conflicts with already-registered route 'GET /b/:x'"));
        // Re-observing under the same id may flip loopback_only.
        table.insert(loopback_route("a", "GET", "/b/:x")).unwrap();
        assert!(
            table
                .match_route_for_listener("GET", "/b/1", REMOTE)
                .is_none()
        );
        table.insert(route("a", "GET", "/b/:x")).unwrap();
        assert!(
            table
                .match_route_for_listener("GET", "/b/1", REMOTE)
                .is_some()
        );
    }

    #[test]
    fn the_unfiltered_lookup_still_sees_loopback_only_routes() {
        // `match_route` / `allowed_methods` are the normal listener over
        // loopback: every non-restricted route.
        let mut table = RouteTable::default();
        table.insert(loopback_route("mcp", "POST", "/mcp")).unwrap();
        assert_eq!(table.match_route("POST", "/mcp").unwrap().0.id(), "mcp");
        assert_eq!(table.allowed_methods("/mcp"), vec!["POST"]);
    }
}
