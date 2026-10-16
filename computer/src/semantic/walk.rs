//! One budgeted, memoized walk over a provider. Children are fetched at most
//! once per parent per walk (real providers mint a fresh handle on every
//! fetch, and xa11y's matcher deduplicates by handle), only fresh fetches
//! count against the node budget, depth is absolute from the walk root, and
//! every node is redacted before anything — the selector matcher included —
//! sees it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use xa11y_core::{ElementData, Provider, Role};

pub struct Budget {
    pub max_nodes: usize,
    pub max_depth: u32,
    pub deadline: Instant,
}

pub struct Walk<'a> {
    provider: &'a dyn Provider,
    budget: Budget,
    children: RefCell<HashMap<u64, Vec<ElementData>>>,
    depth: RefCell<HashMap<u64, u32>>,
    parent: RefCell<HashMap<u64, u64>>,
    nodes: RefCell<HashMap<u64, ElementData>>,
    visited: Cell<usize>,
    truncated: Cell<bool>,
    depth_cut: Cell<bool>,
    root: u64,
    /// The parent whose children could not be listed because it vanished.
    stale_parent: Cell<Option<u64>>,
}

impl<'a> Walk<'a> {
    pub fn new(provider: &'a dyn Provider, root: &ElementData, budget: Budget) -> Self {
        let walk = Self {
            provider,
            budget,
            children: RefCell::new(HashMap::new()),
            depth: RefCell::new(HashMap::new()),
            parent: RefCell::new(HashMap::new()),
            nodes: RefCell::new(HashMap::new()),
            visited: Cell::new(0),
            truncated: Cell::new(false),
            depth_cut: Cell::new(false),
            root: root.handle,
            stale_parent: Cell::new(None),
        };
        walk.depth.borrow_mut().insert(root.handle, 0);
        walk.nodes.borrow_mut().insert(root.handle, root.clone());
        walk
    }

    /// The `get_children` the matcher and the BFS both go through.
    pub fn children(&self, parent: Option<&ElementData>) -> xa11y_core::Result<Vec<ElementData>> {
        let Some(parent) = parent else {
            return Ok(Vec::new());
        };
        if let Some(kids) = self.children.borrow().get(&parent.handle) {
            return Ok(kids.clone());
        }
        let depth = self
            .depth
            .borrow()
            .get(&parent.handle)
            .copied()
            .unwrap_or(self.budget.max_depth);
        if depth >= self.budget.max_depth {
            self.depth_cut.set(true);
            return Ok(Vec::new());
        }
        if Instant::now() >= self.budget.deadline || self.visited.get() >= self.budget.max_nodes {
            self.truncated.set(true);
            return Ok(Vec::new());
        }
        let mut kids = match self.provider.get_children(Some(parent)) {
            Ok(kids) => kids,
            Err(e) => {
                if matches!(e, xa11y_core::Error::ElementStale { .. }) {
                    self.stale_parent.set(Some(parent.handle));
                }
                return Err(e);
            }
        };
        let room = self.budget.max_nodes - self.visited.get();
        if kids.len() > room {
            kids.truncate(room);
            self.truncated.set(true);
        }
        self.visited.set(self.visited.get() + kids.len());
        for kid in &mut kids {
            redact(kid);
            self.depth.borrow_mut().insert(kid.handle, depth + 1);
            self.parent.borrow_mut().insert(kid.handle, parent.handle);
            self.nodes.borrow_mut().insert(kid.handle, kid.clone());
        }
        self.children
            .borrow_mut()
            .insert(parent.handle, kids.clone());
        Ok(kids)
    }

    /// Breadth-first fetch from `root` until the budget, deadline or depth
    /// stops it: under a node budget, breadth covers more of the screen than
    /// the first deep branch would.
    pub fn fill(&self, root: &ElementData) -> xa11y_core::Result<()> {
        let mut queue = VecDeque::from([root.clone()]);
        while let Some(node) = queue.pop_front() {
            queue.extend(self.children(Some(&node))?);
            if self.truncated.get() {
                break;
            }
        }
        Ok(())
    }

    /// What was fetched, parent before children, as `(depth, element)`;
    /// the root itself is left out.
    pub fn preorder(&self, root: &ElementData) -> Vec<(u32, ElementData)> {
        let children = self.children.borrow();
        let depth = self.depth.borrow();
        let mut out = Vec::new();
        let mut stack: Vec<ElementData> = children
            .get(&root.handle)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .rev()
            .collect();
        while let Some(el) = stack.pop() {
            if let Some(kids) = children.get(&el.handle) {
                stack.extend(kids.iter().rev().cloned());
            }
            out.push((depth.get(&el.handle).copied().unwrap_or(1), el));
        }
        out
    }

    /// Up to `n` named ancestors of `handle`, nearest first, excluding the
    /// walk root.
    pub fn ancestors(&self, handle: u64, n: usize) -> Vec<ElementData> {
        let parent = self.parent.borrow();
        let nodes = self.nodes.borrow();
        let mut out = Vec::new();
        let mut cur = handle;
        while out.len() < n {
            let Some(&p) = parent.get(&cur) else { break };
            if !parent.contains_key(&p) {
                break; // `p` is the walk root
            }
            if let Some(data) = nodes.get(&p) {
                if data.name.as_deref().is_some_and(|s| !s.trim().is_empty()) {
                    out.push(data.clone());
                }
            }
            cur = p;
        }
        out
    }

    pub fn truncated(&self) -> bool {
        self.truncated.get()
    }

    /// A node at `max_depth` was asked for its children and got none
    /// because of the depth limit. Not a truncation.
    pub fn depth_cut(&self) -> bool {
        self.depth_cut.get()
    }

    /// The walk failed because its own root vanished, not a descendant.
    pub fn root_vanished(&self) -> bool {
        self.stale_parent.get() == Some(self.root)
    }
}

/// Names that mark a field as secret anywhere inside the name ("NewPassword",
/// "Password2", "Kennwortbestätigung"). Over-redaction is the safe failure.
const SECRET_PARTS: &[&str] = &[
    "password",
    "passcode",
    "passwort",
    "kennwort",
    "contraseña",
    "contrasena",
    "secret",
    "token",
];
/// Short names that only count as a whole word, so "Shipping", "Pinned" and
/// "Spinner" stay readable.
const SECRET_WORDS: &[&str] = &["pin", "senha"];

/// A field the platform itself marks as a password: macOS secure fields by
/// role/subrole; AT-SPI PASSWORD_TEXT by its role name (GTK, Qt) or xa11y's
/// numeric fallback (AccessKit), which xa11y maps to a plain text field.
pub fn secure_role(el: &ElementData) -> bool {
    let raw = |key: &str| el.raw.get(key).and_then(|v| v.as_str());
    raw("ax_role") == Some("AXSecureTextField")
        || raw("ax_subrole") == Some("AXSecureTextField")
        || raw("atspi_role")
            .is_some_and(|r| r.eq_ignore_ascii_case("password text") || r == "role_num:40")
}

pub fn is_secret(el: &ElementData) -> bool {
    if secure_role(el)
        // macOS could not read the subrole (vendor/xa11y-macos M4): it may
        // say AXSecureTextField, so fail closed.
        // ponytail: hides any such node's value, not only a text field's;
        // AppKit answers a missing subrole as absent, so this is rare.
        || el.raw.contains_key("ax_subrole_error")
    {
        return true;
    }
    let Some(name) = el.name.as_deref() else {
        return false;
    };
    // Lowercase and drop combining marks, so a decomposed "contrasen\u{303}a"
    // reads as "contrasena".
    let norm: String = name
        .to_lowercase()
        .chars()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .collect();
    if SECRET_PARTS.iter().any(|p| norm.contains(p)) {
        return true;
    }
    let words: Vec<&str> = norm
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| SECRET_WORDS.contains(w))
        || words.windows(3).any(|w| w == ["mot", "de", "passe"])
}

const VALUE_KEYS: [&str; 3] = ["AXValue", "uia_value", "atspi_value"];

/// The `raw` key a redacted secret field carries when it held text, so the
/// tree can say `(set)` without the value. Never a selector attribute: raw
/// keys are refused there. Linux never sets it: AT-SPI password text is
/// never read (vendor/xa11y-linux P4).
pub const FILLED: &str = "computer_filled";

/// Drop a secret's value everywhere xa11y keeps it. Only the value keys
/// leave `raw`: the Windows backend reads other raw keys while walking.
pub fn redact(el: &mut ElementData) {
    if is_secret(el) {
        let filled = el.value.as_deref().is_some_and(|v| !v.is_empty())
            || VALUE_KEYS.iter().any(|k| {
                el.raw
                    .get(*k)
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| !v.is_empty())
            });
        el.value = None;
        for key in VALUE_KEYS {
            el.raw.remove(key);
        }
        // Only a field holds typed text; a label named "Password:" carries
        // its own name as its value.
        let field = secure_role(el)
            || el.states.editable
            || matches!(el.role, Role::TextField | Role::TextArea | Role::ComboBox);
        if filled && field {
            el.raw.insert(FILLED.into(), true.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::fake::FakeTree;
    use std::time::{Duration, Instant};
    use xa11y_core::Role;

    fn budget(max_nodes: usize, max_depth: u32) -> Budget {
        Budget {
            max_nodes,
            max_depth,
            deadline: Instant::now() + Duration::from_secs(5),
        }
    }

    fn root(tree: &FakeTree) -> ElementData {
        tree.list_apps().unwrap().remove(0)
    }

    #[test]
    fn fill_respects_absolute_depth() {
        let t = FakeTree::new("App", 1);
        let w1 = t.add(0, Role::Window, "W");
        let g2 = t.add(w1, Role::Group, "G");
        let b3 = t.add(g2, Role::Button, "Deep");
        t.add(b3, Role::StaticText, "Deeper");
        let r = root(&t);
        let walk = Walk::new(&t, &r, budget(256, 2));
        walk.fill(&r).unwrap();
        let names: Vec<String> = walk
            .preorder(&r)
            .into_iter()
            .map(|(_, e)| e.name.unwrap())
            .collect();
        assert_eq!(names, vec!["W", "G"]);
        assert!(!walk.truncated(), "a depth cut is not a truncation");
        assert!(walk.depth_cut());
        let shallow = Walk::new(&t, &r, budget(256, 6));
        shallow.fill(&r).unwrap();
        assert!(!shallow.depth_cut(), "the whole tree fits in max_depth");
    }

    #[test]
    fn node_budget_cuts_a_wide_node_and_reports_it() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        for i in 0..300 {
            t.add(w, Role::ListItem, &format!("row {i}"));
        }
        let r = root(&t);
        let walk = Walk::new(&t, &r, budget(256, 6));
        walk.fill(&r).unwrap();
        assert_eq!(walk.preorder(&r).len(), 256);
        assert!(walk.truncated());
    }

    #[test]
    fn an_expired_deadline_fetches_nothing() {
        let t = FakeTree::new("App", 1);
        t.add(0, Role::Window, "W");
        let r = root(&t);
        let walk = Walk::new(
            &t,
            &r,
            Budget {
                max_nodes: 256,
                max_depth: 6,
                deadline: Instant::now(),
            },
        );
        walk.fill(&r).unwrap();
        assert!(walk.preorder(&r).is_empty());
        assert!(walk.truncated());
        assert_eq!(t.children_calls(), 0);
    }

    #[test]
    fn children_are_fetched_once_per_parent_even_with_fresh_handles() {
        let t = FakeTree::new("App", 1).with_fresh_handles();
        let w = t.add(0, Role::Window, "W");
        t.add(w, Role::Button, "OK");
        let r = root(&t);
        let walk = Walk::new(&t, &r, budget(256, 6));
        let first = walk.children(Some(&r)).unwrap();
        let again = walk.children(Some(&r)).unwrap();
        assert_eq!(first[0].handle, again[0].handle);
        assert_eq!(t.children_calls(), 1);
    }

    #[test]
    fn ancestors_are_named_nearest_first_and_stop_before_the_root() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "Doc");
        let g = t.add(w, Role::Group, "");
        let d = t.add(g, Role::Dialog, "Salvar alterações");
        let ok = t.add(d, Role::Button, "OK");
        let _ = ok;
        let r = root(&t);
        let walk = Walk::new(&t, &r, budget(256, 6));
        walk.fill(&r).unwrap();
        let ok_el = walk
            .preorder(&r)
            .into_iter()
            .map(|(_, e)| e)
            .find(|e| e.name.as_deref() == Some("OK"))
            .unwrap();
        let names: Vec<String> = walk
            .ancestors(ok_el.handle, 3)
            .into_iter()
            .map(|e| e.name.unwrap())
            .collect();
        assert_eq!(names, vec!["Salvar alterações", "Doc"]);
    }

    #[test]
    fn atspi_password_roles_are_secret_whatever_the_name() {
        // AT-SPI PASSWORD_TEXT reaches us as a plain text field: the role
        // name where the bridge serves GetRoleName (GTK, Qt), xa11y's numeric
        // fallback where it does not (AccessKit).
        for role in ["password text", "Password Text", "role_num:40"] {
            let mut e = ElementData::for_role(Role::TextField);
            e.name = Some("Código".into());
            e.value = Some("hunter2".into());
            e.raw.insert("atspi_role".into(), role.into());
            e.raw.insert("atspi_value".into(), "hunter2".into());
            redact(&mut e);
            assert_eq!(e.value, None, "{role}");
            assert!(!e.raw.contains_key("atspi_value"), "{role}");
            // Copies of name and role stay: they are not the secret.
            assert!(e.raw.contains_key("atspi_role"), "{role}");
        }
        let mut plain = ElementData::for_role(Role::TextField);
        plain.name = Some("Código".into());
        plain.value = Some("x".into());
        plain.raw.insert("atspi_role".into(), "text".into());
        plain.raw.insert("atspi_value".into(), "x".into());
        redact(&mut plain);
        assert_eq!(plain.value.as_deref(), Some("x"));
        assert!(plain.raw.contains_key("atspi_value"));
    }

    #[test]
    fn secrets_are_redacted_by_subrole_role_and_name() {
        let mut secure = ElementData::for_role(Role::TextField);
        secure.value = Some("hunter2".into());
        secure
            .raw
            .insert("ax_subrole".into(), "AXSecureTextField".into());
        secure.raw.insert("AXValue".into(), "hunter2".into());
        redact(&mut secure);
        assert_eq!(secure.value, None);
        assert!(!secure.raw.contains_key("AXValue"));

        // macOS could not read the subrole: hidden; no subrole at all: kept.
        let mut unread = ElementData::for_role(Role::TextField);
        unread.value = Some("hunter2".into());
        unread.raw.insert("ax_role".into(), "AXTextField".into());
        unread
            .raw
            .insert("ax_subrole_error".into(), (-25204).into());
        unread.raw.insert("AXValue".into(), "hunter2".into());
        redact(&mut unread);
        assert_eq!(unread.value, None);
        assert!(!unread.raw.contains_key("AXValue"));
        let mut plain = ElementData::for_role(Role::TextField);
        plain.value = Some("x".into());
        plain.raw.insert("ax_role".into(), "AXTextField".into());
        plain.raw.insert("AXValue".into(), "x".into());
        redact(&mut plain);
        assert_eq!(plain.value.as_deref(), Some("x"));
        assert!(plain.raw.contains_key("AXValue"));

        let mut by_role = ElementData::for_role(Role::TextField);
        by_role
            .raw
            .insert("ax_role".into(), "AXSecureTextField".into());
        assert!(is_secret(&by_role));

        for name in [
            "Senha",
            "PIN",
            "Password:",
            "API token",
            "Mot de passe",
            "contraseña",
            "NewPassword",
            "Password2",
            "Passwords",
            "Kennwortbestätigung",
            "contrasen\u{303}a",
        ] {
            let mut e = ElementData::for_role(Role::TextField);
            e.name = Some(name.into());
            e.value = Some("x".into());
            e.raw.insert("uia_value".into(), "x".into());
            redact(&mut e);
            assert_eq!(e.value, None, "{name}");
            assert!(!e.raw.contains_key("uia_value"), "{name}");
        }
        for name in ["Shipping", "Pinned", "Nome", "Spinner"] {
            let mut e = ElementData::for_role(Role::TextField);
            e.name = Some(name.into());
            e.value = Some("x".into());
            redact(&mut e);
            assert_eq!(e.value.as_deref(), Some("x"), "{name}");
        }
    }

    #[test]
    fn redaction_happens_before_the_selector_sees_the_node() {
        let t = FakeTree::new("App", 1);
        let w = t.add(0, Role::Window, "W");
        let s = t.add(w, Role::TextField, "Senha");
        t.edit(s, |e| e.value = Some("hunter2".into()));
        let r = root(&t);
        let walk = Walk::new(&t, &r, budget(256, 6));
        let group = xa11y_core::SelectorGroup::parse("text_field[value^='h']").unwrap();
        let found = xa11y_core::selector::find_elements_in_tree_group(
            |p| walk.children(p),
            Some(&r),
            &group,
            None,
            None,
        )
        .unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn only_a_secret_field_holding_text_is_marked_filled() {
        let el = |role: Role, name: &str, value: &str| {
            let mut e = ElementData::for_role(role);
            e.name = Some(name.into());
            e.value = Some(value.into());
            redact(&mut e);
            assert_eq!(e.value, None, "{name}");
            e.raw.contains_key(FILLED)
        };
        assert!(el(Role::TextField, "Senha", "hunter2"));
        assert!(!el(Role::TextField, "Senha", ""));
        // A label carries its own name as its value: never "set".
        assert!(!el(Role::StaticText, "Password:", "Password:"));
        assert!(!el(Role::Button, "Show password", "x"));
    }
}
