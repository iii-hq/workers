//! The synchronous core, run only on a session's executor thread. One active
//! snapshot per session: its provider (whose handle cache backs the refs) and
//! the ref table. A snapshot with results replaces the previous one, which
//! drops that provider and kills its refs.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use xa11y_core::{App, ElementData, Provider, SelectorGroup};

use super::outline::{element_line, label, Outline};
use super::refs::next_ref;
use super::walk::{Budget, Walk};
use super::{map_act_error, map_read_error, Code, ProviderFactory, SemanticError, MAX_FIND_LINES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Invoke,
    SetValue,
    Toggle,
    Select,
}

impl Verb {
    pub fn parse(action: &str) -> Option<Self> {
        match action {
            "invoke" => Some(Self::Invoke),
            "set_value" => Some(Self::SetValue),
            "toggle" => Some(Self::Toggle),
            "select" => Some(Self::Select),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invoke => "invoke",
            Self::SetValue => "set_value",
            Self::Toggle => "toggle",
            Self::Select => "select",
        }
    }

    /// The xa11y action name an element advertises for this verb.
    fn action_name(self) -> &'static str {
        match self {
            Self::Invoke => "press",
            Self::SetValue => "set_value",
            Self::Toggle => "toggle",
            Self::Select => "select",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub app_pid: Option<u32>,
    pub app_name: Option<String>,
    pub root_ref: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Query {
    pub scope: Scope,
    pub selector: Option<SelectorGroup>,
    pub max_nodes: usize,
    pub max_depth: u32,
}

#[derive(Debug, Clone)]
pub struct SnapshotOut {
    /// Tree text for the model (marker line first).
    pub text: String,
    /// Elements listed (observe) or matched in total (find).
    pub matches: usize,
    /// The walk stopped at the node budget or the deadline.
    pub truncated: bool,
}

struct Snapshot {
    provider: Arc<dyn Provider>,
    refs: HashMap<String, ElementData>,
    /// Invalidation counter when the walk began.
    born: u64,
}

const INVALIDATED: &str = "refs were invalidated by a physical action on this desktop";
const NO_SNAPSHOT: &str =
    "no active snapshot (none taken yet, or the accessibility executor was reset)";

pub struct Engine {
    factory: ProviderFactory,
    invalidations: Arc<AtomicU64>,
    active: Option<Snapshot>,
    /// Why there is no active snapshot: never taken, or dropped because a
    /// physical action invalidated it.
    gone: &'static str,
}

fn stale(what: impl Into<String>) -> SemanticError {
    SemanticError::new(Code::StaleRef, what)
}

fn unknown_ref(r: &str) -> SemanticError {
    stale(format!(
        "unknown ref {r} (from a replaced snapshot, or never issued)"
    ))
}

/// The one running app named exactly `name`. `App::by_name_with` would
/// silently take the first of several; an ambiguous name is refused instead.
fn app_named(provider: &dyn Provider, name: &str) -> Result<ElementData, SemanticError> {
    let mut pids = HashSet::new();
    let mut apps: Vec<ElementData> = provider
        .list_apps()
        .map_err(|e| map_read_error(&e, false))?
        .into_iter()
        .filter(|a| a.name.as_deref() == Some(name))
        .filter(|a| a.pid.is_none_or(|p| pids.insert(p)))
        .collect();
    match apps.len() {
        0 => Err(SemanticError::new(
            Code::InvalidInput,
            format!("no running application named \"{name}\""),
        )),
        1 => Ok(apps.remove(0)),
        n => {
            let pids: Vec<String> = apps
                .iter()
                .filter_map(|a| a.pid)
                .map(|p| p.to_string())
                .collect();
            let which = match (pids.len(), n - pids.len()) {
                (_, 0) => format!("pids {}", pids.join(", ")),
                (0, unknown) => format!("{unknown} without a pid"),
                (_, unknown) => format!("pids {} and {unknown} without a pid", pids.join(", ")),
            };
            Err(SemanticError::new(
                Code::InvalidInput,
                format!("{n} running applications are named \"{name}\" ({which}); pass app_pid"),
            ))
        }
    }
}

impl Engine {
    pub fn new(factory: ProviderFactory, invalidations: Arc<AtomicU64>) -> Self {
        Self {
            factory,
            invalidations,
            active: None,
            gone: NO_SNAPSHOT,
        }
    }

    /// Drop an invalidated snapshot now, freeing its provider and handle
    /// cache; later calls still learn why their refs died.
    fn reap(&mut self) {
        let now = self.invalidations.load(Ordering::SeqCst);
        if self.active.as_ref().is_some_and(|s| s.born != now) {
            self.active = None;
            self.gone = INVALIDATED;
        }
    }

    fn live(&self) -> Result<&Snapshot, SemanticError> {
        let snap = self.active.as_ref().ok_or_else(|| stale(self.gone))?;
        if snap.born != self.invalidations.load(Ordering::SeqCst) {
            return Err(stale(INVALIDATED));
        }
        Ok(snap)
    }

    fn resolve_root(
        &self,
        scope: &Scope,
    ) -> Result<(Arc<dyn Provider>, ElementData), SemanticError> {
        if let Some(r) = &scope.root_ref {
            let snap = self.live()?;
            let el = snap.refs.get(r).cloned().ok_or_else(|| unknown_ref(r))?;
            return Ok((snap.provider.clone(), el));
        }
        let provider = (self.factory)().map_err(|e| map_read_error(&e, false))?;
        if let Some(name) = &scope.app_name {
            let app = app_named(provider.as_ref(), name)?;
            return Ok((provider, app));
        }
        let found = match scope.app_pid {
            Some(pid) => App::by_pid_with(provider.clone(), pid, Duration::ZERO),
            None => App::foreground_with(provider.clone(), Duration::ZERO),
        };
        // A lookup that finds no app is `SelectorNotMatched` (xa11y's poll
        // loop makes a single attempt with a zero timeout); anything else,
        // `Timeout` included, is the provider failing.
        let app = found.map_err(|e| match e {
            xa11y_core::Error::SelectorNotMatched { .. } if scope.app_pid.is_some() => {
                SemanticError::new(Code::InvalidInput, "no running application matches app_pid")
            }
            xa11y_core::Error::SelectorNotMatched { .. } => SemanticError::new(
                Code::InvalidInput,
                "no application is in the foreground; pass app_name or app_pid",
            ),
            other => map_read_error(&other, false),
        })?;
        Ok((provider, app.data))
    }

    pub fn snapshot(
        &mut self,
        q: &Query,
        deadline: Instant,
        install_if_empty: bool,
    ) -> Result<SnapshotOut, SemanticError> {
        self.reap();
        let born = self.invalidations.load(Ordering::SeqCst);
        let rooted = q.scope.root_ref.is_some();
        let (provider, root) = self.resolve_root(&q.scope)?;
        let mut out = Outline::default();
        let mut refs = HashMap::new();
        let matches;
        let truncated;
        let depth_cut;
        {
            let walk = Walk::new(
                provider.as_ref(),
                &root,
                Budget {
                    max_nodes: q.max_nodes,
                    max_depth: q.max_depth,
                    deadline,
                },
            );
            match &q.selector {
                None => {
                    walk.fill(&root)
                        .map_err(|e| map_read_error(&e, rooted && walk.root_vanished()))?;
                    for (depth, el) in walk.preorder(&root) {
                        let r = next_ref();
                        let indent = "  ".repeat(depth.saturating_sub(1) as usize);
                        // Only a printed line gets a live ref.
                        if out.push(format!("{indent}{}", element_line(Some(&r), &el))) {
                            refs.insert(r, el);
                        }
                    }
                    matches = refs.len();
                }
                Some(group) => {
                    let found = xa11y_core::selector::find_elements_in_tree_group(
                        |p| walk.children(p),
                        Some(&root),
                        group,
                        None,
                        None,
                    )
                    .map_err(|e| map_read_error(&e, rooted && walk.root_vanished()))?;
                    matches = found.len();
                    for el in found.into_iter().take(MAX_FIND_LINES) {
                        let r = next_ref();
                        let mut line = element_line(Some(&r), &el);
                        for a in walk.ancestors(el.handle, 3) {
                            line.push_str(" ← ");
                            line.push_str(&label(&a));
                        }
                        if out.push(line) {
                            refs.insert(r, el);
                        }
                    }
                    if matches > MAX_FIND_LINES {
                        out.note(format!(
                            "{matches} matches; showing the first {MAX_FIND_LINES} — narrow the selector"
                        ));
                    }
                    if matches == 0 {
                        out.note("no matches".to_string());
                    }
                }
            }
            truncated = walk.truncated();
            depth_cut = walk.depth_cut();
        }
        let empty_find = q.selector.is_some() && matches == 0;
        if truncated && empty_find {
            out.note(
                "zero matches may be a false negative: the walk was cut by the node budget or \
                 deadline; raise max_nodes/max_depth or scope with app_name/root_ref"
                    .to_string(),
            );
        } else if truncated {
            out.note(
                "walk cut by the node budget or deadline; elements beyond it are not listed"
                    .to_string(),
            );
        } else if depth_cut && empty_find {
            out.note(
                "zero matches may be a false negative: the walk reached max_depth; raise \
                 max_depth (up to 12) or scope with root_ref"
                    .to_string(),
            );
        }
        // The root keeps its ref, so find(root_ref) → act → find(root_ref)
        // can confirm under the same root.
        if let Some(r) = &q.scope.root_ref {
            refs.insert(r.clone(), root);
        }
        if matches > 0 || install_if_empty {
            self.active = Some(Snapshot {
                provider,
                refs,
                born,
            });
        }
        Ok(SnapshotOut {
            text: out.finish(),
            matches,
            truncated,
        })
    }

    pub fn act(
        &mut self,
        r: &str,
        verb: Verb,
        text: Option<&str>,
    ) -> Result<String, SemanticError> {
        self.reap();
        let snap = self.live()?;
        let el = snap.refs.get(r).ok_or_else(|| unknown_ref(r))?;
        if !el.states.enabled {
            return Err(SemanticError::new(
                Code::NotSupported,
                format!("{r} was disabled when observed"),
            ));
        }
        let advertised = el.actions.iter().any(|a| a == verb.action_name());
        let p = &snap.provider;
        let result = match (verb, text) {
            (Verb::Invoke, _) => p.press(el),
            (Verb::SetValue, Some(text)) => p.set_value(el, text),
            // Never `set_value("")`: a missing text is a caller bug.
            (Verb::SetValue, None) => {
                return Err(SemanticError::new(
                    Code::InvalidInput,
                    "set_value needs `text`",
                ))
            }
            (Verb::Toggle, _) => p.toggle(el),
            (Verb::Select, _) => p.select(el),
        };
        result.map_err(|e| map_act_error(&e, verb.as_str(), advertised))?;
        Ok(format!(
            "dispatched {} on {r} ({}); effect not verified — confirm at the end with \
             computer::find (it replaces current refs)",
            verb.as_str(),
            label(el)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::fake::FakeTree;
    use xa11y_core::{Error as X, Role};

    fn engine(tree: &Arc<FakeTree>) -> (Engine, Arc<AtomicU64>) {
        let inv = Arc::new(AtomicU64::new(0));
        (Engine::new(tree.factory(), inv.clone()), inv)
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    fn query(selector: Option<&str>) -> Query {
        Query {
            scope: Scope::default(),
            selector: selector.map(|s| crate::semantic::parse_selector(s).unwrap()),
            max_nodes: 256,
            max_depth: 6,
        }
    }

    /// The ref on the first line whose text contains `needle`.
    fn ref_of(text: &str, needle: &str) -> String {
        let line = text
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} in {text}"));
        line.trim_start()
            .strip_prefix("[ref=")
            .unwrap()
            .split(']')
            .next()
            .unwrap()
            .to_string()
    }

    fn form() -> Arc<FakeTree> {
        let t = FakeTree::new("Fixture", 42);
        let w = t.add(0, Role::Window, "Doc");
        t.add(w, Role::Button, "Salvar");
        t.add(w, Role::TextField, "Nome");
        t.add(w, Role::CheckBox, "Ativo");
        let off = t.add(w, Role::Button, "Desabilitado");
        t.edit(off, |e| e.states.enabled = false);
        Arc::new(t)
    }

    #[test]
    fn observe_lists_the_tree_with_refs_but_not_the_app_root() {
        let t = form();
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        let lines: Vec<&str> = out.text.lines().collect();
        assert_eq!(lines[0], crate::semantic::outline::UNTRUSTED);
        assert!(
            lines[1].starts_with("[ref=e") && lines[1].ends_with("window \"Doc\""),
            "{}",
            lines[1]
        );
        assert!(lines[2].starts_with("  [ref=e") && lines[2].contains("button \"Salvar\""));
        assert!(!out.text.contains("application"));
        assert_eq!(out.matches, 5);
        assert!(!out.truncated);
    }

    #[test]
    fn refs_are_unique_across_engines() {
        let t = form();
        let (mut a, _) = engine(&t);
        let (mut b, _) = engine(&t);
        let ra = ref_of(
            &a.snapshot(&query(None), deadline(), true).unwrap().text,
            "Salvar",
        );
        let rb = ref_of(
            &b.snapshot(&query(None), deadline(), true).unwrap().text,
            "Salvar",
        );
        assert_ne!(ra, rb);
        // A ref from another session never resolves here.
        assert_eq!(
            b.act(&ra, Verb::Invoke, None).unwrap_err().code,
            Code::StaleRef
        );
    }

    #[test]
    fn invoke_dispatches_once_and_never_claims_the_effect() {
        let t = form();
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        let r = ref_of(&out.text, "Salvar");
        let detail = e.act(&r, Verb::Invoke, None).unwrap();
        assert_eq!(t.actions(), vec!["press:Salvar"]);
        assert!(detail.contains("effect not verified"), "{detail}");
        assert!(detail.contains("button \"Salvar\""));
        // Semantic acts keep the snapshot: a form can be filled field by field.
        let nome = ref_of(&out.text, "Nome");
        e.act(&nome, Verb::SetValue, Some("ação, João, €")).unwrap();
        assert_eq!(t.actions()[1], "set_value=ação, João, €:Nome");
    }

    #[test]
    fn disabled_elements_are_refused_without_dispatch() {
        let t = form();
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        let r = ref_of(&out.text, "Desabilitado");
        assert_eq!(
            e.act(&r, Verb::Invoke, None).unwrap_err().code,
            Code::NotSupported
        );
        assert!(t.actions().is_empty());
    }

    #[test]
    fn provider_failures_map_to_spec_codes_without_retries() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        let gone = t.add(w, Role::Button, "Gone");
        let advertised = t.add(w, Role::Button, "Removed on Windows");
        let cannot = t.add(w, Role::Button, "Busy app");
        let text = t.add(w, Role::TextField, "Campo");
        let t = Arc::new(t);
        t.fail(
            advertised,
            X::ActionNotSupported {
                action: "press".into(),
                role: Role::Button,
            },
        );
        t.fail(
            cannot,
            X::Platform {
                code: -25204,
                message: "cannot complete".into(),
            },
        );
        t.fail(text, X::TextValueNotSupported);
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        t.remove(gone);
        assert_eq!(
            e.act(&ref_of(&out.text, "Gone"), Verb::Invoke, None)
                .unwrap_err()
                .code,
            Code::StaleRef
        );
        assert_eq!(
            e.act(&ref_of(&out.text, "Removed on Windows"), Verb::Invoke, None)
                .unwrap_err()
                .code,
            Code::StaleRef
        );
        assert_eq!(
            e.act(&ref_of(&out.text, "Busy app"), Verb::Invoke, None)
                .unwrap_err()
                .code,
            Code::EffectUnknown
        );
        assert_eq!(
            e.act(&ref_of(&out.text, "Campo"), Verb::SetValue, Some("x"))
                .unwrap_err()
                .code,
            Code::EffectUnknown
        );
        // One provider call per act: no retry, no fallback.
        assert_eq!(t.actions().len(), 4, "{:?}", t.actions());
    }

    #[test]
    fn only_printed_lines_get_refs() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        for i in 0..200 {
            t.add(w, Role::Button, &format!("{i:03} {}", "x".repeat(400)));
        }
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        assert!(
            out.text.contains("lines omitted (output cap)"),
            "the cap must be reached"
        );
        let printed = out
            .text
            .lines()
            .filter(|l| l.trim_start().starts_with("[ref="))
            .count();
        // `matches` is the ref table's size: one ref per printed line.
        assert_eq!(out.matches, printed);
    }

    #[test]
    fn a_vanished_root_ref_is_stale_but_a_vanished_descendant_is_a_backend_error() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        let d = t.add(w, Role::Dialog, "Opções");
        let g = t.add(d, Role::Group, "Grupo");
        t.add(g, Role::Button, "OK");
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        let mut q = query(None);
        q.scope.root_ref = Some(ref_of(&out.text, "dialog"));
        // The group vanishes between the dialog's fetch and its own.
        let tree = t.clone();
        let fetches = Arc::new(AtomicU64::new(0));
        let n = fetches.clone();
        *t.on_fetch.lock().unwrap() = Some(Box::new(move || {
            if n.fetch_add(1, Ordering::SeqCst) == 1 {
                tree.remove(g);
            }
        }));
        let err = e.snapshot(&q, deadline(), true).unwrap_err();
        *t.on_fetch.lock().unwrap() = None;
        assert_eq!(err.code, Code::BackendError, "{}", err.message);
        // The root_ref element itself going away is the caller's stale ref.
        t.remove(d);
        let err = e.snapshot(&q, deadline(), true).unwrap_err();
        assert_eq!(err.code, Code::StaleRef, "{}", err.message);
        assert!(err.message.contains("root_ref"), "{}", err.message);
    }

    #[test]
    fn a_physical_action_during_the_walk_invalidates_its_refs() {
        let t = form();
        let (mut e, inv) = engine(&t);
        let bump = inv.clone();
        *t.on_fetch.lock().unwrap() = Some(Box::new(move || {
            bump.fetch_add(1, Ordering::SeqCst);
        }));
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        *t.on_fetch.lock().unwrap() = None;
        let err = e
            .act(&ref_of(&out.text, "Salvar"), Verb::Invoke, None)
            .unwrap_err();
        assert_eq!(err.code, Code::StaleRef);
        assert!(err.message.contains("physical action"), "{}", err.message);
        assert!(t.actions().is_empty());
    }

    #[test]
    fn an_invalidated_snapshot_is_dropped_by_the_next_call() {
        let t = form();
        let (mut e, inv) = engine(&t);
        let base = Arc::strong_count(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        assert_eq!(
            Arc::strong_count(&t),
            base + 1,
            "the snapshot holds its provider"
        );
        inv.fetch_add(1, Ordering::SeqCst);
        let none = e
            .snapshot(&query(Some("button[name='Nada']")), deadline(), false)
            .unwrap();
        assert_eq!(none.matches, 0);
        assert_eq!(
            Arc::strong_count(&t),
            base,
            "the invalidated provider is freed"
        );
        let err = e
            .act(&ref_of(&out.text, "Salvar"), Verb::Invoke, None)
            .unwrap_err();
        assert_eq!(err.code, Code::StaleRef);
        assert!(err.message.contains("physical action"), "{}", err.message);
    }

    #[test]
    fn a_new_snapshot_replaces_the_old_refs() {
        let t = form();
        let (mut e, _) = engine(&t);
        let old = ref_of(
            &e.snapshot(&query(None), deadline(), true).unwrap().text,
            "Salvar",
        );
        e.snapshot(&query(Some("button[name='Salvar']")), deadline(), false)
            .unwrap();
        assert_eq!(
            e.act(&old, Verb::Invoke, None).unwrap_err().code,
            Code::StaleRef
        );
    }

    #[test]
    fn find_without_matches_keeps_the_active_snapshot() {
        let t = form();
        let (mut e, _) = engine(&t);
        let r = ref_of(
            &e.snapshot(&query(None), deadline(), true).unwrap().text,
            "Salvar",
        );
        let none = e
            .snapshot(&query(Some("button[name='Nada']")), deadline(), false)
            .unwrap();
        assert_eq!(none.matches, 0);
        assert!(none.text.starts_with(crate::semantic::outline::UNTRUSTED));
        assert!(none.text.contains("no matches"));
        e.act(&r, Verb::Invoke, None).unwrap();
    }

    #[test]
    fn find_lists_each_control_once_even_with_fresh_handles_and_nesting() {
        let t = FakeTree::new("App", 1).with_fresh_handles();
        let w = t.add(0, Role::Window, "W");
        let outer = t.add(w, Role::Group, "outer");
        let inner = t.add(outer, Role::Group, "inner");
        t.add(inner, Role::Button, "OK");
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let out = e
            .snapshot(&query(Some("group button")), deadline(), false)
            .unwrap();
        assert_eq!(out.matches, 1, "{}", out.text);
        assert!(
            out.text
                .contains("button \"OK\" ← group \"inner\" ← group \"outer\" ← window \"W\""),
            "{}",
            out.text
        );
    }

    #[test]
    fn find_caps_listed_matches_and_never_picks_one() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        for _ in 0..60 {
            t.add(w, Role::Button, "OK");
        }
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let out = e
            .snapshot(&query(Some("button[name='OK']")), deadline(), false)
            .unwrap();
        assert_eq!(out.matches, 60);
        assert_eq!(
            out.text.lines().filter(|l| l.starts_with("[ref=")).count(),
            50
        );
        assert!(out.text.contains("60 matches; showing the first 50"));
        assert!(t.actions().is_empty());
    }

    #[test]
    fn root_ref_reuses_the_snapshot_provider_and_honours_invalidation() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        let d = t.add(w, Role::Dialog, "Salvar alterações");
        t.add(d, Role::Button, "OK");
        t.add(w, Role::Button, "OK");
        let t = Arc::new(t);
        let calls = Arc::new(AtomicU64::new(0));
        let counted = calls.clone();
        let tree = t.clone();
        let factory: ProviderFactory = Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(tree.clone() as Arc<dyn Provider>)
        });
        let inv = Arc::new(AtomicU64::new(0));
        let mut e = Engine::new(factory, inv.clone());
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        let dialog = ref_of(&out.text, "dialog");
        let mut q = query(Some("button[name='OK']"));
        q.scope.root_ref = Some(dialog.clone());
        let scoped = e.snapshot(&q, deadline(), false).unwrap();
        assert_eq!(scoped.matches, 1);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "no new provider for root_ref"
        );
        // A coordinate action between observe and the scoped find kills the root.
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        q.scope.root_ref = Some(ref_of(&out.text, "dialog"));
        inv.fetch_add(1, Ordering::SeqCst);
        let err = e.snapshot(&q, deadline(), false).unwrap_err();
        assert_eq!(err.code, Code::StaleRef);
        assert!(err.message.contains("physical action"), "{}", err.message);
    }

    #[test]
    fn scope_errors_are_input_errors() {
        let t = form();
        let (mut e, _) = engine(&t);
        let mut q = query(None);
        q.scope.app_name = Some("Nope".into());
        assert_eq!(
            e.snapshot(&q, deadline(), true).unwrap_err().code,
            Code::InvalidInput
        );
        q.scope.app_name = Some("Bad\"Name".into());
        assert_eq!(
            e.snapshot(&q, deadline(), true).unwrap_err().code,
            Code::InvalidInput
        );
        q.scope.app_name = None;
        q.scope.app_pid = Some(7);
        assert_eq!(
            e.snapshot(&q, deadline(), true).unwrap_err().code,
            Code::InvalidInput
        );
        q.scope.app_pid = Some(42);
        assert!(e.snapshot(&q, deadline(), true).is_ok());
    }

    #[test]
    fn a_cut_walk_says_zero_matches_may_be_false() {
        let t = form();
        let (mut e, _) = engine(&t);
        let mut q = query(Some("button[name='Salvar']"));
        q.max_nodes = 1;
        let out = e.snapshot(&q, deadline(), false).unwrap();
        assert_eq!(out.matches, 0);
        assert!(out.truncated);
        assert!(
            out.text.contains("zero matches may be a false negative"),
            "{}",
            out.text
        );
    }

    #[test]
    fn permission_errors_surface_from_the_factory() {
        let factory: ProviderFactory = Arc::new(|| {
            Err(X::PermissionDenied {
                instructions: "enable Accessibility".into(),
            })
        });
        let mut e = Engine::new(factory, Arc::new(AtomicU64::new(0)));
        let err = e.snapshot(&query(None), deadline(), true).unwrap_err();
        assert_eq!(err.code, Code::PermissionDenied);
        assert!(err.message.contains("enable Accessibility"));
        if cfg!(target_os = "macos") {
            assert!(err.message.contains("Screen Recording"), "{}", err.message);
        } else {
            assert!(!err.message.contains("macOS"), "{}", err.message);
        }
    }

    #[test]
    fn an_ambiguous_app_name_is_refused_and_app_pid_picks_one() {
        let t = FakeTree::new("Doc", 7);
        t.add(0, Role::Window, "First");
        let second = t.add_app("Doc", 8);
        t.add(second, Role::Window, "Second");
        let other = t.add_app("Other", 9);
        t.add(other, Role::Window, "Third");
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let mut q = query(None);
        q.scope.app_name = Some("Doc".into());
        let err = e.snapshot(&q, deadline(), true).unwrap_err();
        assert_eq!(err.code, Code::InvalidInput);
        assert!(
            err.message.contains("2 running applications are named")
                && err.message.contains("pids 7, 8")
                && err.message.contains("pass app_pid"),
            "{}",
            err.message
        );
        q.scope.app_name = None;
        q.scope.app_pid = Some(8);
        let out = e.snapshot(&q, deadline(), true).unwrap();
        assert!(
            out.text.contains("Second") && !out.text.contains("First"),
            "{}",
            out.text
        );
        q.scope.app_pid = None;
        q.scope.app_name = Some("Other".into());
        assert!(e
            .snapshot(&q, deadline(), true)
            .unwrap()
            .text
            .contains("Third"));
        q.scope.app_name = Some("doc".into());
        let err = e.snapshot(&q, deadline(), true).unwrap_err();
        assert_eq!(err.code, Code::InvalidInput);
        assert!(
            err.message.contains("no running application named"),
            "{}",
            err.message
        );
    }

    #[test]
    fn a_root_ref_stays_usable_across_its_own_scoped_finds() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        let d = t.add(w, Role::Dialog, "Opções");
        t.add(d, Role::CheckBox, "Ativo");
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        let dialog = ref_of(&out.text, "dialog");
        let mut q = query(Some("check_box[name='Ativo']"));
        q.scope.root_ref = Some(dialog);
        // find(root_ref) → act → find(root_ref) to confirm.
        let found = e.snapshot(&q, deadline(), false).unwrap();
        e.act(&ref_of(&found.text, "Ativo"), Verb::Toggle, None)
            .unwrap();
        let again = e.snapshot(&q, deadline(), false).unwrap();
        assert_eq!(again.matches, 1, "{}", again.text);
        assert_eq!(t.actions(), vec!["toggle:Ativo"]);
    }

    #[test]
    fn set_value_without_text_is_refused_without_dispatch() {
        let t = form();
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        let err = e
            .act(&ref_of(&out.text, "Nome"), Verb::SetValue, None)
            .unwrap_err();
        assert_eq!(err.code, Code::InvalidInput);
        assert!(t.actions().is_empty(), "{:?}", t.actions());
    }

    #[test]
    fn a_depth_cut_find_with_zero_matches_says_it_may_be_false() {
        let t = form();
        let (mut e, _) = engine(&t);
        // The buttons sit at depth 2; a depth-1 walk never reaches them.
        let mut q = query(Some("button[name='Salvar']"));
        q.max_depth = 1;
        let out = e.snapshot(&q, deadline(), false).unwrap();
        assert_eq!(out.matches, 0);
        assert!(!out.truncated, "a depth cut is not a truncation");
        assert!(
            out.text.contains(
                "zero matches may be a false negative: the walk reached max_depth; raise \
                 max_depth (up to 12) or scope with root_ref"
            ),
            "{}",
            out.text
        );
        // The whole tree fits in the default depth: no such note.
        let out = e
            .snapshot(&query(Some("button[name='Nada']")), deadline(), false)
            .unwrap();
        assert!(!out.text.contains("false negative"), "{}", out.text);
    }

    #[test]
    fn redacted_values_never_reach_observe_or_find_text() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "Login");
        let senha = t.add(w, Role::TextField, "Senha");
        let new_pw = t.add(w, Role::TextField, "NewPassword");
        let nome = t.add(w, Role::TextField, "Nome");
        for (idx, v) in [(senha, "hunter2"), (new_pw, "s3cr3t!"), (nome, "João")] {
            t.edit(idx, |e| {
                e.value = Some(v.into());
                e.raw.insert("AXValue".into(), v.into());
                e.raw.insert("uia_value".into(), v.into());
            });
        }
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let observed = e.snapshot(&query(None), deadline(), true).unwrap().text;
        let found = e
            .snapshot(&query(Some("text_field")), deadline(), false)
            .unwrap();
        assert_eq!(found.matches, 3, "{}", found.text);
        for text in [&observed, &found.text] {
            assert!(
                !text.contains("hunter2") && !text.contains("s3cr3t!"),
                "{text}"
            );
            assert!(text.contains("text_field \"Senha\""), "{text}");
            // A field that is not secret keeps its value.
            assert!(text.contains("value=\"João\""), "{text}");
        }
    }

    #[test]
    fn act_without_a_snapshot_is_stale() {
        let t = form();
        let (mut e, _) = engine(&t);
        assert_eq!(
            e.act("e1", Verb::Invoke, None).unwrap_err().code,
            Code::StaleRef
        );
    }

    #[test]
    fn a_provider_timeout_on_the_app_lookup_is_a_backend_error() {
        let t = form();
        *t.foreground_error.lock().unwrap() = Some(X::timeout(Duration::from_secs(1)));
        let (mut e, _) = engine(&t);
        assert_eq!(
            e.snapshot(&query(None), deadline(), true).unwrap_err().code,
            Code::BackendError
        );
        *t.foreground_error.lock().unwrap() = Some(X::selector_not_matched("focused application"));
        let err = e.snapshot(&query(None), deadline(), true).unwrap_err();
        assert_eq!(err.code, Code::InvalidInput);
        assert!(
            err.message.contains("no application is in the foreground"),
            "{}",
            err.message
        );
    }

    #[test]
    fn an_ambiguous_name_without_pids_never_prints_question_marks() {
        let t = FakeTree::new("Doc", 7);
        let a = t.add_app("Doc", 0);
        let b = t.add_app("Doc", 0);
        for idx in [a, b] {
            t.edit(idx, |e| e.pid = None);
        }
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let mut q = query(None);
        q.scope.app_name = Some("Doc".into());
        let err = e.snapshot(&q, deadline(), true).unwrap_err();
        assert!(
            err.message
                .contains("3 running applications are named \"Doc\" (pids 7 and 2 without a pid)"),
            "{}",
            err.message
        );
        assert!(!err.message.contains('?'), "{}", err.message);
    }

    #[test]
    fn toggle_and_select_reach_their_own_provider_calls() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        t.add(w, Role::CheckBox, "Ativo");
        t.add(w, Role::ListItem, "Dois");
        let t = Arc::new(t);
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        e.act(&ref_of(&out.text, "Ativo"), Verb::Toggle, None)
            .unwrap();
        e.act(&ref_of(&out.text, "Dois"), Verb::Select, None)
            .unwrap();
        assert_eq!(t.actions(), vec!["toggle:Ativo", "select:Dois"]);
    }

    #[test]
    fn an_unadvertised_verb_the_provider_refuses_is_not_supported() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        // A text field advertises only set_value, so a refused press is the
        // element's answer, not a removed element.
        let nome = t.add(w, Role::TextField, "Nome");
        let t = Arc::new(t);
        t.fail(
            nome,
            X::ActionNotSupported {
                action: "press".into(),
                role: Role::TextField,
            },
        );
        let (mut e, _) = engine(&t);
        let out = e.snapshot(&query(None), deadline(), true).unwrap();
        assert_eq!(
            e.act(&ref_of(&out.text, "Nome"), Verb::Invoke, None)
                .unwrap_err()
                .code,
            Code::NotSupported
        );
    }
}
