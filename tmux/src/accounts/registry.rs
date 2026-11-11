//! `accounts.json`: the accounts people added, the global one, the rules
//! and what the rules last did (agent-accounts D2). Without the file there
//! is one account, `default`, and it is global.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::DEFAULT;
use crate::model::{AccountSwitch, SwitchRules};

/// Colour tokens (agent-accounts D15); the apps map them.
pub const COLORS: [&str; 5] = ["blue", "green", "violet", "orange", "pink"];
/// The longest account id.
const ID_MAX: usize = 32;
/// The longest account name.
const NAME_MAX: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Groups shared with `~/.claude` (never for `default`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub created_at_ms: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// A suggestion someone said "Not now" to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dismissal {
    pub from: String,
    pub reason: String,
    /// When `from`'s window resets; the snooze ends then.
    pub until_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default = "version_one")]
    pub version: u32,
    #[serde(default = "default_id")]
    pub global: String,
    #[serde(default)]
    pub accounts: Vec<Entry>,
    #[serde(default)]
    pub rules: SwitchRules,
    #[serde(default)]
    pub dismissed: Vec<Dismissal>,
    #[serde(default)]
    pub last_switch: Option<AccountSwitch>,
}

fn version_one() -> u32 {
    1
}

fn default_id() -> String {
    DEFAULT.to_string()
}

pub fn default_entry() -> Entry {
    Entry {
        id: DEFAULT.to_string(),
        name: "Claude Code".to_string(),
        color: "blue".to_string(),
        shared: Vec::new(),
        created_at_ms: 0,
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: 1,
            global: default_id(),
            accounts: vec![default_entry()],
            rules: SwitchRules::default(),
            dismissed: Vec::new(),
            last_switch: None,
        }
    }
}

impl Registry {
    /// `default` first and present, ids unique, the global one existing,
    /// unknown colours gray at the edges (the apps' job), the try-order
    /// naming accounts that exist.
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.accounts.retain(|entry| {
            // An id becomes a folder name and a tmux option value: one that
            // could name another folder (`..`, `/`) or the word `global` is
            // dropped, whatever a hand edit wrote.
            if !valid_id(&entry.id) {
                tracing::warn!(id = %entry.id, "accounts.json: invalid account id ignored");
                return false;
            }
            seen.insert(entry.id.clone())
        });
        match self.accounts.iter().position(|entry| entry.id == DEFAULT) {
            Some(0) => {}
            Some(at) => {
                let entry = self.accounts.remove(at);
                self.accounts.insert(0, entry);
            }
            None => self.accounts.insert(0, default_entry()),
        }
        self.accounts[0].shared.clear();
        if !self.has(&self.global) {
            self.global = default_id();
        }
        let ids: Vec<String> = self.accounts.iter().map(|e| e.id.clone()).collect();
        let mut order_seen = std::collections::HashSet::new();
        self.rules
            .order
            .retain(|id| ids.contains(id) && order_seen.insert(id.clone()));
    }

    pub fn has(&self, id: &str) -> bool {
        self.accounts.iter().any(|entry| entry.id == id)
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.accounts.iter().find(|entry| entry.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.accounts.iter_mut().find(|entry| entry.id == id)
    }

    /// A new id for `name`: its slug, made unique among the accounts, the
    /// reserved words and what `also_taken` says (a folder left behind).
    pub fn new_id(&self, name: &str, also_taken: impl Fn(&str) -> bool) -> String {
        let base = slug(name);
        let taken =
            |id: &str| self.has(id) || id == DEFAULT || id == super::GLOBAL || also_taken(id);
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|n| {
                let suffix = format!("-{n}");
                let keep = ID_MAX - suffix.len();
                let stem: String = base.chars().take(keep).collect();
                format!("{}{suffix}", stem.trim_end_matches('-'))
            })
            .find(|id| !taken(id))
            .expect("some suffix is free")
    }
}

/// An id an entry may have: `default`, or 1 to 32 of `[a-z0-9-]` starting
/// with a letter or digit, never `global`.
pub fn valid_id(id: &str) -> bool {
    id == DEFAULT
        || (id != super::GLOBAL
            && (1..=ID_MAX).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !id.starts_with('-'))
}

/// `[a-z0-9-]`, at most 32 characters, never empty.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        let c = match c {
            'a'..='z' | '0'..='9' => c,
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            _ => '-',
        };
        if c == '-' && (out.is_empty() || out.ends_with('-')) {
            continue;
        }
        out.push(c);
    }
    let mut out: String = out.chars().take(ID_MAX).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "account".to_string()
    } else {
        out
    }
}

/// A name people can give an account.
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the account a name.".to_string());
    }
    if name.chars().count() > NAME_MAX {
        return Err(format!(
            "An account name has at most {NAME_MAX} characters."
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("An account name can't hold control characters.".to_string());
    }
    Ok(name.to_string())
}

pub fn validate_color(color: &str) -> Result<String, String> {
    if COLORS.contains(&color) {
        Ok(color.to_string())
    } else {
        Err(format!(
            "The colour is one of {}, not \"{color}\".",
            COLORS.join(", ")
        ))
    }
}

/// `accounts.json` on disk, read again when it changes.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    /// `(modified, len, inode)` of the file read, `None` when it was
    /// missing. Each write renames a new file into place: a new inode.
    stamp: Option<(SystemTime, u64, u64)>,
    loaded: bool,
    /// The registry, or why the file could not be read.
    state: Result<Option<Registry>, String>,
    /// One more each time the file read is another version of it (written
    /// by this process or another): a registry read at the same generation
    /// is still the one on disk, even when the same values came back
    /// (A → B → A).
    generation: u64,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            stamp: None,
            loaded: false,
            state: Ok(None),
            generation: 0,
        }
    }

    /// Where changes take their advisory lock.
    pub fn lock_path(registry_file: &Path) -> PathBuf {
        registry_file.with_file_name("accounts.lock")
    }

    fn refresh(&mut self) {
        use std::os::unix::fs::MetadataExt;
        let stamp = std::fs::metadata(&self.path)
            .ok()
            .and_then(|meta| Some((meta.modified().ok()?, meta.len(), meta.ino())));
        if self.loaded && stamp == self.stamp {
            return;
        }
        // A forced read of the same file (under the lock) is no new version.
        if self.generation == 0 || stamp != self.stamp {
            self.generation += 1;
        }
        self.loaded = true;
        self.stamp = stamp;
        self.state = match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str::<Registry>(&text)
                .map(|mut registry| {
                    registry.normalize();
                    Some(registry)
                })
                .map_err(|error| {
                    tracing::warn!(%error, "accounts.json unreadable");
                    format!("accounts.json can't be read ({error}); fix or remove it.")
                }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("accounts.json can't be read ({error}).")),
        };
    }

    /// The file exists and reads.
    pub fn managed(&mut self) -> bool {
        self.refresh();
        matches!(self.state, Ok(Some(_)))
    }

    /// The registry, the implicit one when the file is missing or unreadable.
    pub fn current(&mut self) -> Registry {
        self.current_at().0
    }

    /// The registry with the generation it was read at.
    pub fn current_at(&mut self) -> (Registry, u64) {
        self.refresh();
        let registry = match &self.state {
            Ok(Some(registry)) => registry.clone(),
            _ => Registry::default(),
        };
        (registry, self.generation)
    }

    /// Change the registry and write it, taking the lock itself (see
    /// [`Store::update_locked`]).
    pub fn update<T>(
        &mut self,
        change: impl FnOnce(&mut Registry) -> Result<T, String>,
    ) -> Result<T, String> {
        let lock = FileLock::take(&Self::lock_path(&self.path))?;
        self.update_locked(&lock, |registry, _| change(registry))
    }

    /// Change the registry and write it; refused while the file is
    /// unreadable, so a hand edit gone wrong is never overwritten. `lock`
    /// is `accounts.lock` (an advisory lock), so another process changing
    /// it at the same time waits for this one; the file is read again
    /// under it, and `change` gets the generation of what it read.
    pub fn update_locked<T>(
        &mut self,
        _lock: &FileLock,
        change: impl FnOnce(&mut Registry, u64) -> Result<T, String>,
    ) -> Result<T, String> {
        self.loaded = false;
        self.refresh();
        let mut registry = match &self.state {
            Ok(Some(registry)) => registry.clone(),
            Ok(None) => Registry::default(),
            Err(error) => return Err(error.clone()),
        };
        let out = change(&mut registry, self.generation)?;
        registry.normalize();
        write_atomic(&self.path, &registry)?;
        self.loaded = false;
        self.refresh();
        Ok(out)
    }
}

/// An exclusive `flock` on a file, released when dropped (or when the
/// process ends).
pub struct FileLock(std::fs::File);

/// How long a change of `accounts.json` waits for another process's.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

impl FileLock {
    /// Blocking: waits up to two seconds for another process's change.
    pub fn take(path: &Path) -> Result<Self, String> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        if let Some(folder) = path.parent() {
            make_private_dir(folder)?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(|error| format!("Could not open {} ({error}).", path.display()))?;
        let started = std::time::Instant::now();
        loop {
            // SAFETY: flock on a descriptor this function owns.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self(file));
            }
            if started.elapsed() > LOCK_WAIT {
                return Err("accounts.json is being changed by another process; try again.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: as above; closing the file would release it too.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// `value` as pretty JSON at `path`: a temporary file next to it, `0600`,
/// renamed over it; the folder is made `0700` when missing.
pub fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let folder = path
        .parent()
        .ok_or_else(|| "The accounts folder has no parent.".to_string())?;
    make_private_dir(folder)?;
    let mut text = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    text.push(b'\n');
    let temp = folder.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file"),
        uuid::Uuid::new_v4().simple()
    ));
    let written = (|| {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&text)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Could not save {} ({error}).", path.display()));
    }
    Ok(())
}

/// Make `dir` (and its parents) when missing; a folder made here is `0700`.
pub fn make_private_dir(dir: &Path) -> Result<(), String> {
    if dir.is_dir() {
        return Ok(());
    }
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|error| format!("Could not make {} ({error}).", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RulesMode, Threshold};

    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tmux-accounts-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn slugs_are_short_lowercase_and_dashed() {
        assert_eq!(slug("Side project"), "side-project");
        assert_eq!(slug("  Ação  Pessoal! "), "acao-pessoal");
        assert_eq!(slug("---"), "account");
        assert_eq!(slug(&"x".repeat(50)).len(), 32);
        assert_eq!(slug("Work / 2026"), "work-2026");
    }

    #[test]
    fn new_ids_skip_taken_and_reserved_ones() {
        let mut registry = Registry::default();
        assert_eq!(registry.new_id("Default", |_| false), "default-2");
        assert_eq!(registry.new_id("global", |_| false), "global-2");
        registry.accounts.push(Entry {
            id: "personal".into(),
            name: "Personal".into(),
            color: "green".into(),
            shared: vec![],
            created_at_ms: 1,
        });
        assert_eq!(registry.new_id("Personal", |_| false), "personal-2");
        let long = "y".repeat(40);
        registry.accounts.push(Entry {
            id: slug(&long),
            name: long.clone(),
            color: "green".into(),
            shared: vec![],
            created_at_ms: 1,
        });
        let id = registry.new_id(&long, |_| false);
        assert_eq!(
            registry.new_id("Side project", |id| id == "side-project"),
            "side-project-2"
        );
        assert_eq!(id.len(), 32);
        assert!(id.ends_with("-2"));
    }

    #[test]
    fn normalize_keeps_default_first_and_the_global_real() {
        let mut registry: Registry = serde_json::from_str(
            r#"{"global":"gone","accounts":[{"id":"p","name":"P","color":"green","shared":["settings"]},{"id":"p","name":"dup","color":"pink"}],
                "rules":{"mode":"switch","order":["p","x","p"]}}"#,
        )
        .unwrap();
        registry.normalize();
        assert_eq!(
            registry
                .accounts
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["default", "p"]
        );
        assert_eq!(registry.global, "default");
        assert_eq!(registry.rules.mode, RulesMode::Switch);
        assert_eq!(registry.rules.at, Threshold::Week90, "a partial rule set");
        assert!(registry.rules.move_idle);
        assert_eq!(registry.rules.order, vec!["p"]);
    }

    #[test]
    fn ids_from_disk_are_checked() {
        let ids = [
            "ok-1".to_string(),
            "../x".into(),
            "a/b".into(),
            "global".into(),
            "-lead".into(),
            "UPPER".into(),
            String::new(),
            "y".repeat(33),
        ];
        let accounts: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| serde_json::json!({"id": id, "name": "N", "color": "green"}))
            .collect();
        let mut registry: Registry =
            serde_json::from_value(serde_json::json!({"global": "../x", "accounts": accounts}))
                .unwrap();
        registry.normalize();
        assert_eq!(
            registry
                .accounts
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["default", "ok-1"]
        );
        assert_eq!(registry.global, "default");
        assert!(valid_id("default") && valid_id("side-project-2"));
        assert!(!valid_id("global") && !valid_id("a.b") && !valid_id(".."));
    }

    #[test]
    fn changes_wait_for_another_process() {
        let dir = temp_dir("lock");
        let path = dir.join("accounts.json");
        let held = FileLock::take(&dir.join("accounts.lock")).unwrap();
        let mut store = Store::new(path.clone());
        let started = std::time::Instant::now();
        let busy = store.update(|_| Ok(()));
        assert!(busy.is_err(), "another holder: refused after the wait");
        assert!(started.elapsed() >= LOCK_WAIT);
        drop(held);
        // Another process wrote meanwhile: the change starts from its file.
        let mut other = Store::new(path.clone());
        other
            .update(|registry| {
                registry.rules.five_hour = true;
                Ok(())
            })
            .unwrap();
        store.current();
        store
            .update(|registry| {
                assert!(registry.rules.five_hour, "read again under the lock");
                Ok(())
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_version_of_the_file_is_a_new_generation() {
        let dir = temp_dir("generation");
        let path = dir.join("accounts.json");
        let mut store = Store::new(path.clone());
        let (_, missing) = store.current_at();
        assert_eq!(store.current_at().1, missing, "nothing changed");
        let set = |store: &mut Store, global: &str| {
            let global = global.to_string();
            store
                .update(move |registry| {
                    registry.global = global;
                    Ok(())
                })
                .unwrap();
        };
        set(&mut store, "default");
        let (first, at_first) = store.current_at();
        assert!(at_first > missing);
        // Another process: A → B → A comes back with the same values, not
        // the same generation.
        let mut other = Store::new(path.clone());
        set(&mut other, "personal");
        set(&mut other, "default");
        let (again, at_again) = store.current_at();
        assert_eq!(again.global, first.global);
        assert_ne!(at_again, at_first);
        // A read under the lock of the same file is the same generation.
        let lock = FileLock::take(&Store::lock_path(&path)).unwrap();
        let seen = store
            .update_locked(&lock, |_, generation| Ok(generation))
            .unwrap();
        assert_eq!(seen, at_again);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_brief_example_reads() {
        let text = r#"{ "version": 1,
          "global": "default",
          "accounts": [
            { "id": "default", "name": "Work", "color": "blue" },
            { "id": "personal", "name": "Personal", "color": "green",
              "shared": ["settings","extensions","plugins","conversations"], "created_at_ms": 0 } ],
          "rules": { "mode": "suggest", "at": "week_90", "five_hour": false, "move_idle": true,
                     "back_after_reset": false, "order": ["personal"] },
          "dismissed": [ { "from": "default", "reason": "week_90", "until_ms": 0 } ],
          "last_switch": null }"#;
        let mut registry: Registry = serde_json::from_str(text).unwrap();
        registry.normalize();
        assert_eq!(registry.accounts[0].name, "Work");
        assert_eq!(registry.accounts[1].shared.len(), 4);
        assert_eq!(registry.dismissed[0].reason, "week_90");
    }

    #[test]
    fn the_store_writes_atomically_and_privately() {
        let dir = temp_dir("store");
        let path = dir.join("nested/accounts.json");
        let mut store = Store::new(path.clone());
        assert!(!store.managed());
        assert_eq!(store.current().global, "default");
        store
            .update(|registry| {
                registry.accounts.push(Entry {
                    id: "personal".into(),
                    name: "Personal".into(),
                    color: "green".into(),
                    shared: vec![],
                    created_at_ms: 5,
                });
                registry.global = "personal".into();
                Ok(())
            })
            .unwrap();
        assert!(store.managed());
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let folder = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(folder, 0o700);
        let mut again = Store::new(path.clone());
        assert_eq!(again.current().global, "personal");
        // A hand edit gone wrong is reported and never overwritten.
        std::fs::write(&path, "{ not json").unwrap();
        let mut broken = Store::new(path.clone());
        assert!(!broken.managed());
        assert!(broken.update(|_| Ok(())).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_and_colours_are_checked() {
        assert_eq!(validate_name("  Work ").unwrap(), "Work");
        assert!(validate_name(" ").is_err());
        assert!(validate_name(&"n".repeat(41)).is_err());
        assert!(validate_color("violet").is_ok());
        assert!(validate_color("red").is_err());
    }
}
