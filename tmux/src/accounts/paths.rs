//! Where Claude Code accounts, their sign-ins and Codex's records live.
//!
//! The global slot (agent-accounts G1) is `~/.claude` with `~/.claude.json`
//! next to it: Claude Code runs there with `CLAUDE_CONFIG_DIR` unset (set to
//! `~/.claude`, it would keep its `.claude.json` inside the folder and, on
//! macOS, look for its sign-in under another Keychain name). Every account,
//! `default` included, has a folder of its own under the accounts root
//! (G2), whose `.claude.json` is inside it: it holds the account's sign-in
//! while another account is global.
//!
//! For tests and demos (the README's "For tests and demos"):
//! `III_CLAUDE_HOME` replaces `~/.claude`, and the default account's
//! `.claude.json` is then the one next to it, in its parent folder, as
//! `~/.claude.json` is next to `~/.claude`;
//! `III_CLAUDE_ACCOUNTS_DIR` replaces the accounts root;
//! `III_CLAUDE_USAGE_URL` replaces `https://api.anthropic.com` for the usage
//! endpoint (loopback addresses only, so a sign-in never leaves for another
//! host); `CODEX_HOME` is Codex's own.

use std::path::{Path, PathBuf};

/// The real usage endpoint's origin.
pub const ANTHROPIC_API: &str = "https://api.anthropic.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudePaths {
    /// The home folder, for `~` in what people read.
    pub home: PathBuf,
    /// The global slot: `~/.claude`.
    pub claude_home: PathBuf,
    /// The slot's `.claude.json`: `~/.claude.json`.
    pub default_json: PathBuf,
    /// Where added accounts live; `accounts.json` is in it.
    pub root: PathBuf,
    /// Codex's folder: `~/.codex`.
    pub codex_home: PathBuf,
    /// The worker's usage cache file, when there is a cache folder.
    pub cache_file: Option<PathBuf>,
    /// Origin of the usage endpoint.
    pub usage_origin: String,
}

impl ClaudePaths {
    /// From this process's environment; `None` without a home folder.
    pub fn from_env() -> Option<Self> {
        Self::from_vars(
            |key| std::env::var(key).ok().filter(|value| !value.is_empty()),
            cfg!(target_os = "macos"),
        )
    }

    /// From `get` (an environment lookup); `macos` picks the cache folder.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>, macos: bool) -> Option<Self> {
        let absolute = |key: &str| get(key).map(PathBuf::from).filter(|p| p.is_absolute());
        let home = absolute("HOME")?;
        let (claude_home, default_json) = match absolute("III_CLAUDE_HOME") {
            Some(dir) => {
                let json = dir
                    .parent()
                    .map(|parent| parent.join(".claude.json"))
                    .unwrap_or_else(|| home.join(".claude.json"));
                (dir, json)
            }
            None => (home.join(".claude"), home.join(".claude.json")),
        };
        let root = absolute("III_CLAUDE_ACCOUNTS_DIR").unwrap_or_else(|| {
            absolute("XDG_DATA_HOME")
                .unwrap_or_else(|| home.join(".local/share"))
                .join("iii/claude-accounts")
        });
        let codex_home = absolute("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
        let cache = if macos {
            Some(home.join("Library/Caches"))
        } else {
            Some(absolute("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")))
        };
        let usage_origin = get("III_CLAUDE_USAGE_URL")
            .map(|url| url.trim_end_matches('/').to_string())
            .unwrap_or_else(|| ANTHROPIC_API.to_string());
        Some(Self {
            home,
            claude_home,
            default_json,
            root,
            codex_home,
            cache_file: cache.map(|cache| cache.join("iii/tmux/usage.json")),
            usage_origin,
        })
    }

    /// `accounts.json`.
    pub fn registry_file(&self) -> PathBuf {
        self.root.join("accounts.json")
    }

    /// The folder of account `id` (`default` too): `<root>/<id>` (G2).
    pub fn account_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// The `.claude.json` of account `id`'s folder.
    pub fn account_json(&self, id: &str) -> PathBuf {
        self.root.join(id).join(".claude.json")
    }

    /// `path` with the home folder as `~`.
    pub fn display(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }

    /// Whether `path` (a process's `CLAUDE_CONFIG_DIR`, or its default) is
    /// the global slot: `~/.claude`, with or without a trailing slash.
    pub fn is_slot_dir(&self, path: &Path) -> bool {
        same_dir(path, &self.claude_home)
    }
}

/// Two folder paths that name the same folder: equal once trailing slashes
/// and `.` parts are dropped, or the same folder on disk.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    let clean = |path: &Path| path.components().collect::<PathBuf>();
    if clean(a) == clean(b) {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The usage endpoint's origin as a URL, when a sign-in may be sent there:
/// the real one, or a loopback address for tests and demos (agent-accounts
/// D11). Parsed, so `http://127.0.0.1:8787@evil.example` (userinfo before
/// another host) is refused; no userinfo, query or fragment, nothing but
/// `/` as its path.
pub fn allowed_origin(origin: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(origin).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return None;
    }
    let real = url.scheme() == "https"
        && url.host_str() == Some("api.anthropic.com")
        && url.port().is_none();
    // The parser writes hosts in their canonical form (`127.1` is
    // `127.0.0.1`, IPv6 in brackets).
    let loopback = matches!(url.scheme(), "http" | "https")
        && matches!(url.host_str()?, "localhost" | "127.0.0.1" | "[::1]");
    (real || loopback).then_some(url)
}

/// Whether the usage endpoint may be sent a sign-in (see [`allowed_origin`]).
pub fn origin_allowed(origin: &str) -> bool {
    allowed_origin(origin).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn paths(vars: &[(&str, &str)], macos: bool) -> Option<ClaudePaths> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ClaudePaths::from_vars(|key| vars.get(key).cloned(), macos)
    }

    #[test]
    fn defaults_follow_the_home_folder() {
        let p = paths(&[("HOME", "/home/dev")], false).unwrap();
        assert_eq!(p.claude_home, Path::new("/home/dev/.claude"));
        assert_eq!(p.default_json, Path::new("/home/dev/.claude.json"));
        assert_eq!(
            p.root,
            Path::new("/home/dev/.local/share/iii/claude-accounts")
        );
        assert_eq!(p.codex_home, Path::new("/home/dev/.codex"));
        assert_eq!(
            p.cache_file.as_deref(),
            Some(Path::new("/home/dev/.cache/iii/tmux/usage.json"))
        );
        assert_eq!(p.usage_origin, ANTHROPIC_API);
        assert_eq!(
            p.account_dir("default"),
            Path::new("/home/dev/.local/share/iii/claude-accounts/default")
        );
        assert_eq!(
            p.account_json("work"),
            Path::new("/home/dev/.local/share/iii/claude-accounts/work/.claude.json")
        );
        assert!(paths(&[], false).is_none(), "no home, no accounts");
    }

    #[test]
    fn overrides_win() {
        let p = paths(
            &[
                ("HOME", "/home/dev"),
                ("XDG_DATA_HOME", "/data"),
                ("XDG_CACHE_HOME", "/cache"),
                ("III_CLAUDE_HOME", "/t/fake/.claude"),
                ("CODEX_HOME", "/t/codex"),
                ("III_CLAUDE_USAGE_URL", "http://127.0.0.1:8787/"),
            ],
            false,
        )
        .unwrap();
        assert_eq!(p.claude_home, Path::new("/t/fake/.claude"));
        assert_eq!(p.default_json, Path::new("/t/fake/.claude.json"));
        assert_eq!(p.root, Path::new("/data/iii/claude-accounts"));
        assert_eq!(p.codex_home, Path::new("/t/codex"));
        assert_eq!(
            p.cache_file.as_deref(),
            Some(Path::new("/cache/iii/tmux/usage.json"))
        );
        assert_eq!(p.usage_origin, "http://127.0.0.1:8787");
        let p = paths(
            &[("HOME", "/Users/me"), ("III_CLAUDE_ACCOUNTS_DIR", "/t/acc")],
            true,
        )
        .unwrap();
        assert_eq!(p.root, Path::new("/t/acc"));
        assert_eq!(
            p.cache_file.as_deref(),
            Some(Path::new("/Users/me/Library/Caches/iii/tmux/usage.json"))
        );
    }

    #[test]
    fn home_reads_as_a_tilde() {
        let p = paths(&[("HOME", "/home/dev")], false).unwrap();
        assert_eq!(p.display(Path::new("/home/dev/.claude")), "~/.claude");
        assert_eq!(p.display(Path::new("/home/dev")), "~");
        assert_eq!(p.display(Path::new("/srv/x")), "/srv/x");
        assert!(p.is_slot_dir(Path::new("/home/dev/.claude/")));
        assert!(p.is_slot_dir(Path::new("/home/dev/./.claude")));
        assert!(!p.is_slot_dir(Path::new("/home/dev/.claude-work")));
    }

    #[test]
    fn a_sign_in_only_goes_to_anthropic_or_loopback() {
        assert!(origin_allowed("https://api.anthropic.com"));
        assert!(origin_allowed("http://127.0.0.1:8787"));
        assert!(origin_allowed("http://localhost:9"));
        assert!(origin_allowed("http://[::1]:9"));
        assert!(!origin_allowed("https://api.anthropic.com.evil.dev"));
        assert!(!origin_allowed("http://10.0.0.2:8787"));
        assert!(!origin_allowed("http://127.0.0.1.nip.io"));
        assert!(!origin_allowed("ftp://127.0.0.1"));
        // Userinfo before another host: the host is evil.example.
        assert!(!origin_allowed("http://127.0.0.1:8787@evil.example"));
        assert!(!origin_allowed("http://user@127.0.0.1:8787"));
        assert!(!origin_allowed("https://api.anthropic.com:8443"));
        assert!(!origin_allowed("http://api.anthropic.com"));
        assert!(!origin_allowed("http://127.0.0.1:8787/elsewhere"));
        assert!(!origin_allowed("http://127.0.0.1:8787/?x=1"));
        assert!(!origin_allowed("http://127.0.0.2:8787"));
        assert!(origin_allowed("https://api.anthropic.com/"));
    }
}
