//! Test-only `Provider`: a tree the test shapes, an action log, scripted
//! errors, hang/panic switches and an optional "fresh handle per fetch" mode
//! that mirrors the real macOS/Windows providers.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use xa11y_core::{ElementData, Error, Provider, Result, Role, ShellSurfaceKind, Subscription};

use super::ProviderFactory;

struct Node {
    data: ElementData,
    children: Vec<usize>,
}

pub(crate) struct FakeTree {
    nodes: Mutex<Vec<Node>>,
    /// Application roots, in `list_apps` order; node 0 first.
    apps: Mutex<Vec<usize>>,
    removed: Mutex<HashSet<usize>>,
    errors: Mutex<HashMap<usize, Error>>,
    handle_to_node: Mutex<HashMap<u64, usize>>,
    fresh_handles: bool,
    next_handle: AtomicU64,
    log: Mutex<Vec<String>>,
    fetches: AtomicUsize,
    pub block_reads: AtomicBool,
    pub block_actions: AtomicBool,
    pub panic_on_action: AtomicBool,
    pub on_fetch: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// When set, `focused_app` fails with this error: no app in the
    /// foreground (`SelectorNotMatched`) or a provider failure.
    pub foreground_error: Mutex<Option<Error>>,
}

fn default_actions(role: Role) -> Vec<String> {
    let names: &[&str] = match role {
        Role::Button | Role::Link | Role::MenuItem => &["press"],
        Role::TextField | Role::TextArea => &["set_value"],
        Role::CheckBox | Role::Switch | Role::RadioButton => &["press", "toggle"],
        Role::ListItem | Role::Tab => &["press", "select"],
        _ => &[],
    };
    names.iter().map(|s| s.to_string()).collect()
}

fn app_node(name: &str, pid: u32) -> Node {
    let mut data = ElementData::for_role(Role::Application);
    data.name = Some(name.to_string());
    data.pid = Some(pid);
    Node {
        data,
        children: Vec::new(),
    }
}

impl FakeTree {
    /// Node 0 is the Application `app` with process id `pid`; it is also
    /// the focused app.
    pub fn new(app: &str, pid: u32) -> Self {
        Self {
            nodes: Mutex::new(vec![app_node(app, pid)]),
            apps: Mutex::new(vec![0]),
            removed: Mutex::new(HashSet::new()),
            errors: Mutex::new(HashMap::new()),
            handle_to_node: Mutex::new(HashMap::new()),
            fresh_handles: false,
            next_handle: AtomicU64::new(1_000),
            log: Mutex::new(Vec::new()),
            fetches: AtomicUsize::new(0),
            block_reads: AtomicBool::new(false),
            block_actions: AtomicBool::new(false),
            panic_on_action: AtomicBool::new(false),
            on_fetch: Mutex::new(None),
            foreground_error: Mutex::new(None),
        }
    }

    /// Mint a new handle on every fetch, like AX/UIA do.
    pub fn with_fresh_handles(mut self) -> Self {
        self.fresh_handles = true;
        self
    }

    /// Another running application root; `list_apps` returns every one.
    pub fn add_app(&self, name: &str, pid: u32) -> usize {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.push(app_node(name, pid));
        let idx = nodes.len() - 1;
        self.apps.lock().unwrap().push(idx);
        idx
    }

    pub fn add(&self, parent: usize, role: Role, name: &str) -> usize {
        let mut nodes = self.nodes.lock().unwrap();
        let mut data = ElementData::for_role(role);
        if !name.is_empty() {
            data.name = Some(name.to_string());
        }
        data.actions = default_actions(role);
        let idx = nodes.len();
        nodes.push(Node {
            data,
            children: Vec::new(),
        });
        nodes[parent].children.push(idx);
        idx
    }

    pub fn edit(&self, idx: usize, f: impl FnOnce(&mut ElementData)) {
        f(&mut self.nodes.lock().unwrap()[idx].data);
    }

    /// Detach from its parent; actions on a handle already handed out fail
    /// with `ElementStale`, like a destroyed AX element.
    pub fn remove(&self, idx: usize) {
        let mut nodes = self.nodes.lock().unwrap();
        for node in nodes.iter_mut() {
            node.children.retain(|c| *c != idx);
        }
        self.removed.lock().unwrap().insert(idx);
    }

    /// Make every action on `idx` fail with `err`.
    pub fn fail(&self, idx: usize, err: Error) {
        self.errors.lock().unwrap().insert(idx, err);
    }

    pub fn actions(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    pub fn children_calls(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }

    pub fn factory(self: &Arc<Self>) -> ProviderFactory {
        let tree = self.clone();
        Arc::new(move || Ok(tree.clone() as Arc<dyn Provider>))
    }

    fn handle_for(&self, idx: usize) -> u64 {
        let handle = if self.fresh_handles {
            self.next_handle.fetch_add(1, Ordering::SeqCst)
        } else {
            idx as u64 + 1
        };
        self.handle_to_node.lock().unwrap().insert(handle, idx);
        handle
    }

    fn data(&self, idx: usize) -> ElementData {
        let mut data = self.nodes.lock().unwrap()[idx].data.clone();
        data.handle = self.handle_for(idx);
        data
    }

    fn idx(&self, el: &ElementData) -> Result<usize> {
        self.handle_to_node
            .lock()
            .unwrap()
            .get(&el.handle)
            .copied()
            .ok_or(Error::ElementStale {
                selector: format!("handle:{}", el.handle),
            })
    }

    /// Every action call is logged on entry — before it hangs, panics or
    /// fails — so "exactly one dispatch" and "never dispatched" are both
    /// visible in `actions()`.
    fn act(&self, el: &ElementData, what: String) -> Result<()> {
        let idx = self.idx(el);
        let name = match &idx {
            Ok(i) => self.nodes.lock().unwrap()[*i]
                .data
                .name
                .clone()
                .unwrap_or_default(),
            Err(_) => "<unknown handle>".to_string(),
        };
        self.log.lock().unwrap().push(format!("{what}:{name}"));
        if self.panic_on_action.load(Ordering::SeqCst) {
            panic!("fake provider panicked on {what}");
        }
        while self.block_actions.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let idx = idx?;
        if self.removed.lock().unwrap().contains(&idx) {
            return Err(Error::ElementStale {
                selector: format!("node:{idx}"),
            });
        }
        if let Some(err) = self.errors.lock().unwrap().get(&idx) {
            return Err(err.clone());
        }
        Ok(())
    }
}

fn unsupported<T>(what: &str) -> Result<T> {
    Err(Error::Unsupported {
        feature: format!("fake provider: {what}"),
    })
}

impl Provider for FakeTree {
    fn get_children(&self, element: Option<&ElementData>) -> Result<Vec<ElementData>> {
        let Some(element) = element else {
            return self.list_apps();
        };
        self.fetches.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.on_fetch.lock().unwrap().as_ref() {
            hook();
        }
        while self.block_reads.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let idx = self.idx(element)?;
        // A destroyed element can no longer list its children.
        if self.removed.lock().unwrap().contains(&idx) {
            return Err(Error::ElementStale {
                selector: format!("node:{idx}"),
            });
        }
        let kids = self.nodes.lock().unwrap()[idx].children.clone();
        Ok(kids.into_iter().map(|k| self.data(k)).collect())
    }
    fn get_parent(&self, _: &ElementData) -> Result<Option<ElementData>> {
        unsupported("get_parent")
    }
    fn list_apps(&self) -> Result<Vec<ElementData>> {
        let apps = self.apps.lock().unwrap().clone();
        Ok(apps.into_iter().map(|a| self.data(a)).collect())
    }
    fn focused_app(&self) -> Result<ElementData> {
        if let Some(e) = self.foreground_error.lock().unwrap().clone() {
            return Err(e);
        }
        Ok(self.data(0))
    }
    fn list_shell_surfaces(&self) -> Result<Vec<(ShellSurfaceKind, ElementData)>> {
        Ok(Vec::new())
    }
    fn press(&self, el: &ElementData) -> Result<()> {
        self.act(el, "press".into())
    }
    fn focus(&self, _: &ElementData) -> Result<()> {
        unsupported("focus")
    }
    fn blur(&self, _: &ElementData) -> Result<()> {
        unsupported("blur")
    }
    fn toggle(&self, el: &ElementData) -> Result<()> {
        self.act(el, "toggle".into())
    }
    fn select(&self, el: &ElementData) -> Result<()> {
        self.act(el, "select".into())
    }
    fn expand(&self, _: &ElementData) -> Result<()> {
        unsupported("expand")
    }
    fn collapse(&self, _: &ElementData) -> Result<()> {
        unsupported("collapse")
    }
    fn show_menu(&self, _: &ElementData) -> Result<()> {
        unsupported("show_menu")
    }
    fn increment(&self, _: &ElementData) -> Result<()> {
        unsupported("increment")
    }
    fn decrement(&self, _: &ElementData) -> Result<()> {
        unsupported("decrement")
    }
    fn scroll_into_view(&self, _: &ElementData) -> Result<()> {
        unsupported("scroll_into_view")
    }
    fn set_value(&self, el: &ElementData, value: &str) -> Result<()> {
        self.act(el, format!("set_value={value}"))
    }
    fn set_numeric_value(&self, _: &ElementData, _: f64) -> Result<()> {
        unsupported("set_numeric_value")
    }
    fn type_text(&self, _: &ElementData, _: &str) -> Result<()> {
        unsupported("type_text")
    }
    fn set_text_selection(&self, _: &ElementData, _: u32, _: u32) -> Result<()> {
        unsupported("set_text_selection")
    }
    fn perform_action(&self, _: &ElementData, action: &str) -> Result<()> {
        unsupported(action)
    }
    fn activate(&self, _: &ElementData) -> Result<()> {
        unsupported("activate")
    }
    fn minimize(&self, _: &ElementData) -> Result<()> {
        unsupported("minimize")
    }
    fn maximize(&self, _: &ElementData) -> Result<()> {
        unsupported("maximize")
    }
    fn enter_fullscreen(&self, _: &ElementData) -> Result<()> {
        unsupported("enter_fullscreen")
    }
    fn restore(&self, _: &ElementData) -> Result<()> {
        unsupported("restore")
    }
    fn close(&self, _: &ElementData) -> Result<()> {
        unsupported("close")
    }
    fn move_to(&self, _: &ElementData, _: i32, _: i32) -> Result<()> {
        unsupported("move_to")
    }
    fn resize_to(&self, _: &ElementData, _: u32, _: u32) -> Result<()> {
        unsupported("resize_to")
    }
    fn subscribe(&self, _: &ElementData) -> Result<Subscription> {
        unsupported("subscribe")
    }
}
