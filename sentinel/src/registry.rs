//! Who owns the function that failed, and which version was running.
//!
//! The span cannot answer either question. Its `service_name` is whoever
//! emitted it — often the engine's own `call` hop, when the callee's span has
//! not arrived yet — so attributing by it files the group under `iii` and
//! points the investigation at the wrong repository. The registry answers
//! instead: `engine::functions::list` says which worker registered the
//! function, and `engine::workers::list` says which version of it is
//! connected.
//!
//! The cache is keyed by `(namespace, function_id)` rather than by the
//! function id alone. The engine allows the same id once per namespace —
//! `FunctionSummary.namespace` is described as the key a function lives
//! under — so a single-key cache would let one registration overwrite the
//! other and attribute failures to whichever worker registered last. When the
//! namespace cannot be determined, the ambiguity is kept and shown rather
//! than resolved by guessing.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::SentinelError;

/// The namespace recorded when the owner could not be pinned down. It is a
/// value of its own so an ambiguous group never mixes with a resolved one.
pub const AMBIGUOUS_NAMESPACE: &str = "?";
pub const DEFAULT_NAMESPACE: &str = "default";

/// Refreshes are rate-limited: a storm of unknown functions must not become a
/// storm of registry calls.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionEntry {
    pub function_id: String,
    pub namespace: String,
    pub worker_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerEntry {
    pub name: String,
    pub namespace: String,
    pub version: Option<String>,
}

/// The two registry reads this needs.
#[async_trait]
pub trait EngineRegistry: Send + Sync {
    async fn list_functions(&self) -> Result<Vec<FunctionEntry>, SentinelError>;
    async fn list_workers(&self) -> Result<Vec<WorkerEntry>, SentinelError>;
}

/// The worker a failure belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub namespace: String,
    pub worker: String,
    pub version: Option<String>,
    /// The function exists in more than one namespace and nothing chose
    /// between them. The occurrence is marked, not filed under a guess.
    pub ambiguous: bool,
}

#[derive(Debug, Default)]
struct Snapshot {
    /// `(namespace, function_id)` → owning worker.
    owners: HashMap<(String, String), String>,
    /// `function_id` → the namespaces it is registered in.
    namespaces: HashMap<String, BTreeSet<String>>,
    /// `(namespace, worker)` → self-reported version.
    versions: HashMap<(String, String), Option<String>>,
    /// Worker name → the namespaces it runs in.
    worker_namespaces: HashMap<String, BTreeSet<String>>,
    fetched_at: Option<Instant>,
}

pub struct Registry<E: EngineRegistry> {
    engine: E,
    snapshot: RwLock<Snapshot>,
    ttl: Duration,
}

impl<E: EngineRegistry> Registry<E> {
    pub fn new(engine: E, ttl: Duration) -> Self {
        Self {
            engine,
            snapshot: RwLock::new(Snapshot::default()),
            ttl,
        }
    }

    /// Resolve the owner of a failing span.
    ///
    /// `span_namespace` is whatever the SDK stamped, which today is nothing
    /// for Rust workers; `span_service` is the alias-resolved service name,
    /// used only to break a tie.
    pub async fn resolve(
        &self,
        span_namespace: Option<&str>,
        function_id: Option<&str>,
        span_service: &str,
    ) -> Owner {
        self.refresh_if_stale(function_id, span_service).await;
        let snapshot = self.snapshot.read().await;
        resolve_in(&snapshot, span_namespace, function_id, span_service)
    }

    /// Force the next resolve to re-read the registry.
    pub async fn invalidate(&self) {
        self.snapshot.write().await.fetched_at = None;
    }

    async fn refresh_if_stale(&self, function_id: Option<&str>, span_service: &str) {
        let needs = {
            let snapshot = self.snapshot.read().await;
            match snapshot.fetched_at {
                None => true,
                Some(fetched) if fetched.elapsed() >= self.ttl => true,
                // A function or worker nobody has heard of is worth one
                // refresh, but not more often than the floor.
                Some(fetched) if fetched.elapsed() >= MIN_REFRESH_INTERVAL => {
                    let unknown_function =
                        function_id.is_some_and(|id| !snapshot.namespaces.contains_key(id));
                    let unknown_worker = !snapshot.worker_namespaces.contains_key(span_service);
                    unknown_function || unknown_worker
                }
                Some(_) => false,
            }
        };
        if !needs {
            return;
        }

        let functions = self.engine.list_functions().await;
        let workers = self.engine.list_workers().await;
        let (Ok(functions), Ok(workers)) = (functions, workers) else {
            // A registry that cannot be read leaves the last snapshot in
            // place: stale attribution beats no attribution.
            tracing::warn!("could not refresh the function registry; using the last snapshot");
            return;
        };

        let mut snapshot = self.snapshot.write().await;
        *snapshot = build(functions, workers);
    }
}

fn build(functions: Vec<FunctionEntry>, workers: Vec<WorkerEntry>) -> Snapshot {
    let mut snapshot = Snapshot {
        fetched_at: Some(Instant::now()),
        ..Snapshot::default()
    };
    for entry in functions {
        snapshot
            .namespaces
            .entry(entry.function_id.clone())
            .or_default()
            .insert(entry.namespace.clone());
        snapshot
            .owners
            .insert((entry.namespace, entry.function_id), entry.worker_name);
    }
    for worker in workers {
        snapshot
            .worker_namespaces
            .entry(worker.name.clone())
            .or_default()
            .insert(worker.namespace.clone());
        snapshot
            .versions
            .insert((worker.namespace, worker.name), worker.version);
    }
    snapshot
}

fn resolve_in(
    snapshot: &Snapshot,
    span_namespace: Option<&str>,
    function_id: Option<&str>,
    span_service: &str,
) -> Owner {
    let Some(function_id) = function_id.filter(|id| !id.is_empty()) else {
        // No function to look up: the span's own service is the best answer,
        // and its namespace is only known if the name is unique.
        return owner_from_service(snapshot, span_namespace, span_service);
    };

    // 1. The span said which namespace it is in.
    if let Some(namespace) = span_namespace.filter(|value| !value.is_empty()) {
        if let Some(worker) = snapshot
            .owners
            .get(&(namespace.to_string(), function_id.to_string()))
        {
            return owner(snapshot, namespace, worker, false);
        }
    }

    let candidates = snapshot.namespaces.get(function_id);
    match candidates.map(BTreeSet::len) {
        // 2. Registered in exactly one namespace.
        Some(1) => {
            let namespace = candidates
                .and_then(|set| set.iter().next())
                .expect("one candidate");
            let worker = snapshot
                .owners
                .get(&(namespace.clone(), function_id.to_string()))
                .cloned()
                .unwrap_or_else(|| span_service.to_string());
            owner(snapshot, namespace, &worker, false)
        }
        // 3. Several namespaces: the span's service breaks the tie only if it
        //    owns the function in exactly one of them.
        Some(_) => {
            let matching: Vec<&String> = candidates
                .into_iter()
                .flatten()
                .filter(|namespace| {
                    snapshot
                        .owners
                        .get(&((*namespace).clone(), function_id.to_string()))
                        .is_some_and(|worker| worker == span_service)
                })
                .collect();
            if matching.len() == 1 {
                return owner(snapshot, matching[0], span_service, false);
            }
            Owner {
                namespace: AMBIGUOUS_NAMESPACE.to_string(),
                worker: span_service.to_string(),
                version: None,
                ambiguous: true,
            }
        }
        // The registry has never heard of this function.
        None => owner_from_service(snapshot, span_namespace, span_service),
    }
}

fn owner_from_service(
    snapshot: &Snapshot,
    span_namespace: Option<&str>,
    span_service: &str,
) -> Owner {
    if let Some(namespace) = span_namespace.filter(|value| !value.is_empty()) {
        return owner(snapshot, namespace, span_service, false);
    }
    match snapshot.worker_namespaces.get(span_service) {
        Some(namespaces) if namespaces.len() == 1 => {
            let namespace = namespaces.iter().next().expect("one namespace");
            owner(snapshot, namespace, span_service, false)
        }
        Some(_) => Owner {
            namespace: AMBIGUOUS_NAMESPACE.to_string(),
            worker: span_service.to_string(),
            version: None,
            ambiguous: true,
        },
        // Nothing known at all: assume the single-namespace engine this most
        // often is.
        None => Owner {
            namespace: DEFAULT_NAMESPACE.to_string(),
            worker: span_service.to_string(),
            version: None,
            ambiguous: false,
        },
    }
}

fn owner(snapshot: &Snapshot, namespace: &str, worker: &str, ambiguous: bool) -> Owner {
    Owner {
        namespace: namespace.to_string(),
        worker: worker.to_string(),
        version: snapshot
            .versions
            .get(&(namespace.to_string(), worker.to_string()))
            .cloned()
            .flatten(),
        ambiguous,
    }
}

/// Whether a self-reported version is too stale to mean anything. A worker
/// running from a development build reports the same string for weeks, so
/// regression-by-version would never fire — the caller substitutes the
/// checkout's commit instead.
pub fn version_is_indistinct(version: Option<&str>) -> bool {
    match version {
        None => true,
        Some(value) => value.trim().is_empty() || value.ends_with("-dev"),
    }
}

/// A map of the namespaces a worker runs in, for diagnostics.
pub async fn namespaces_of<E: EngineRegistry>(
    registry: &Registry<E>,
    worker: &str,
) -> BTreeMap<String, Option<String>> {
    let snapshot = registry.snapshot.read().await;
    snapshot
        .worker_namespaces
        .get(worker)
        .into_iter()
        .flatten()
        .map(|namespace| {
            (
                namespace.clone(),
                snapshot
                    .versions
                    .get(&(namespace.clone(), worker.to_string()))
                    .cloned()
                    .flatten(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeEngine {
        functions: Vec<FunctionEntry>,
        workers: Vec<WorkerEntry>,
        reads: AtomicUsize,
    }

    impl FakeEngine {
        fn new(functions: Vec<FunctionEntry>, workers: Vec<WorkerEntry>) -> Self {
            Self {
                functions,
                workers,
                reads: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl EngineRegistry for FakeEngine {
        async fn list_functions(&self) -> Result<Vec<FunctionEntry>, SentinelError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.functions.clone())
        }
        async fn list_workers(&self) -> Result<Vec<WorkerEntry>, SentinelError> {
            Ok(self.workers.clone())
        }
    }

    fn function(id: &str, namespace: &str, worker: &str) -> FunctionEntry {
        FunctionEntry {
            function_id: id.into(),
            namespace: namespace.into(),
            worker_name: worker.into(),
        }
    }

    fn worker(name: &str, namespace: &str, version: Option<&str>) -> WorkerEntry {
        WorkerEntry {
            name: name.into(),
            namespace: namespace.into(),
            version: version.map(str::to_string),
        }
    }

    fn registry(engine: FakeEngine) -> Registry<FakeEngine> {
        Registry::new(engine, Duration::from_secs(60))
    }

    #[tokio::test]
    async fn the_owner_of_a_function_wins_over_the_service_that_emitted_the_span() {
        let registry = registry(FakeEngine::new(
            vec![function("state::compare-and-set", "default", "state")],
            vec![worker("state", "default", Some("0.23.0"))],
        ));

        // The callee's own span has not arrived; the engine's `call` hop is
        // what failed, and it belongs to `iii`.
        let owner = registry
            .resolve(None, Some("state::compare-and-set"), "iii")
            .await;
        assert_eq!(owner.worker, "state");
        assert_eq!(owner.namespace, "default");
        assert_eq!(owner.version.as_deref(), Some("0.23.0"));
        assert!(!owner.ambiguous);
    }

    #[tokio::test]
    async fn the_same_function_in_two_namespaces_does_not_overwrite_one_owner() {
        let registry = registry(FakeEngine::new(
            vec![
                function("app::handle", "team-a", "app"),
                function("app::handle", "team-b", "fork"),
            ],
            vec![
                worker("app", "team-a", Some("1.0.0")),
                worker("fork", "team-b", Some("2.0.0")),
            ],
        ));

        // With the namespace known, each resolves to its own owner.
        let a = registry
            .resolve(Some("team-a"), Some("app::handle"), "app")
            .await;
        assert_eq!(a.worker, "app");
        assert_eq!(a.version.as_deref(), Some("1.0.0"));

        let b = registry
            .resolve(Some("team-b"), Some("app::handle"), "fork")
            .await;
        assert_eq!(b.worker, "fork");
        assert_eq!(b.version.as_deref(), Some("2.0.0"));
    }

    #[tokio::test]
    async fn without_a_namespace_the_service_breaks_the_tie_when_it_can() {
        let registry = registry(FakeEngine::new(
            vec![
                function("app::handle", "team-a", "app"),
                function("app::handle", "team-b", "fork"),
            ],
            vec![
                worker("app", "team-a", Some("1.0.0")),
                worker("fork", "team-b", Some("2.0.0")),
            ],
        ));

        let resolved = registry.resolve(None, Some("app::handle"), "fork").await;
        assert_eq!(resolved.namespace, "team-b");
        assert_eq!(resolved.worker, "fork");
        assert!(!resolved.ambiguous);
    }

    #[tokio::test]
    async fn an_unbreakable_tie_is_kept_as_ambiguity_rather_than_guessed() {
        let registry = registry(FakeEngine::new(
            vec![
                function("app::handle", "team-a", "app"),
                function("app::handle", "team-b", "app"),
            ],
            vec![
                worker("app", "team-a", Some("1.0.0")),
                worker("app", "team-b", Some("2.0.0")),
            ],
        ));

        let resolved = registry.resolve(None, Some("app::handle"), "app").await;
        assert!(resolved.ambiguous);
        assert_eq!(
            resolved.namespace, AMBIGUOUS_NAMESPACE,
            "an ambiguous group must never mix with a resolved one"
        );
        assert_eq!(resolved.version, None, "no version can be claimed either");
    }

    #[tokio::test]
    async fn a_span_with_no_function_falls_back_to_its_own_service() {
        let registry = registry(FakeEngine::new(
            vec![],
            vec![worker("harness", "default", Some("1.7.0"))],
        ));
        let resolved = registry.resolve(None, None, "harness").await;
        assert_eq!(resolved.worker, "harness");
        assert_eq!(resolved.namespace, "default");
        assert_eq!(resolved.version.as_deref(), Some("1.7.0"));
    }

    #[tokio::test]
    async fn an_unknown_worker_is_assumed_to_be_in_the_default_namespace() {
        let registry = registry(FakeEngine::new(vec![], vec![]));
        let resolved = registry.resolve(None, None, "brand-new").await;
        assert_eq!(resolved.namespace, DEFAULT_NAMESPACE);
        assert_eq!(resolved.worker, "brand-new");
        assert!(!resolved.ambiguous);
    }

    #[tokio::test]
    async fn the_registry_is_read_once_and_reused_until_it_goes_stale() {
        let engine = FakeEngine::new(
            vec![function("state::cas", "default", "state")],
            vec![worker("state", "default", Some("0.23.0"))],
        );
        let registry = registry(engine);

        for _ in 0..5 {
            registry.resolve(None, Some("state::cas"), "iii").await;
        }
        assert_eq!(
            registry.engine.reads.load(Ordering::SeqCst),
            1,
            "a hot path must not call the registry per occurrence"
        );

        registry.invalidate().await;
        registry.resolve(None, Some("state::cas"), "iii").await;
        assert_eq!(registry.engine.reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_development_version_is_not_distinct_enough_to_detect_a_regression() {
        assert!(version_is_indistinct(None));
        assert!(version_is_indistinct(Some("")));
        assert!(version_is_indistinct(Some("0.23.1-dev")));
        assert!(!version_is_indistinct(Some("0.23.1")));
        assert!(!version_is_indistinct(Some("git:4662b0d")));
    }
}
