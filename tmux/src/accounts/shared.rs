//! What an added account shares with `~/.claude` (agent-accounts D17):
//! symlinks from the account's folder to the default account's items, made
//! when missing and never in place of a real file. Removing a group removes
//! only the links the worker made (those that point at `~/.claude/<item>`).
//!
//! An item `~/.claude` doesn't have yet is not linked (a dangling link to a
//! folder would make Claude Code fail to write there); it is linked the next
//! time the account's links are checked, at worker start or when the account
//! changes.
//!
//! MCP servers live in `.claude.json`, which is each account's own and is
//! never linked: with `plugins`, the user-scope `mcpServers` of
//! `~/.claude.json` are copied once into the new account's `.claude.json`.

use std::path::Path;

use serde_json::Value;

/// The groups, in the order the apps show them.
pub const GROUPS: [&str; 4] = ["settings", "extensions", "plugins", "conversations"];

/// Never shared: each account's sign-in, profile, running sessions and
/// caches.
pub const NEVER: [&str; 5] = [
    ".credentials.json",
    ".claude.json",
    "sessions",
    "statsig",
    "shell-snapshots",
];

/// The items of `group`.
pub fn items(group: &str) -> &'static [&'static str] {
    match group {
        "settings" => &["settings.json", "CLAUDE.md", "keybindings.json"],
        "extensions" => &["skills", "agents", "commands", "output-styles"],
        "plugins" => &["plugins"],
        "conversations" => &["projects", "history.jsonl", "todos", "plans"],
        _ => &[],
    }
}

/// Known groups, each once, in [`GROUPS`] order.
pub fn validate_groups(groups: &[String]) -> Result<Vec<String>, String> {
    if let Some(unknown) = groups.iter().find(|g| !GROUPS.contains(&g.as_str())) {
        return Err(format!(
            "\"{unknown}\" is not something accounts share; use {}.",
            GROUPS.join(", ")
        ));
    }
    Ok(GROUPS
        .iter()
        .filter(|group| groups.iter().any(|g| g == *group))
        .map(|group| group.to_string())
        .collect())
}

/// What linking found, item by item.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Linked {
    /// Links made now.
    pub made: Vec<String>,
    /// Already linked by the worker.
    pub linked: Vec<String>,
    /// A file, folder or other link of the account's own is there: kept.
    pub own: Vec<String>,
    /// `~/.claude` has no such item yet.
    pub missing: Vec<String>,
}

fn is_our_link(link: &Path, target: &Path) -> bool {
    std::fs::symlink_metadata(link).is_ok_and(|meta| meta.file_type().is_symlink())
        && std::fs::read_link(link).is_ok_and(|points| points == target)
}

/// Link `groups`' items of `account_dir` to `claude_home`'s.
pub fn link(account_dir: &Path, claude_home: &Path, groups: &[String]) -> Result<Linked, String> {
    let mut out = Linked::default();
    for item in groups.iter().flat_map(|group| items(group)) {
        debug_assert!(!NEVER.contains(item));
        let link = account_dir.join(item);
        let target = claude_home.join(item);
        match std::fs::symlink_metadata(&link) {
            Ok(_) if is_our_link(&link, &target) => out.linked.push(item.to_string()),
            Ok(_) => out.own.push(item.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(&target).is_err() {
                    out.missing.push(item.to_string());
                    continue;
                }
                std::os::unix::fs::symlink(&target, &link)
                    .map_err(|error| format!("Could not link {} ({error}).", link.display()))?;
                out.made.push(item.to_string());
            }
            Err(error) => return Err(format!("Could not look at {} ({error}).", link.display())),
        }
    }
    Ok(out)
}

/// Remove the links of `groups`' items that the worker made; anything else
/// stays. Returns the items unlinked.
pub fn unlink(
    account_dir: &Path,
    claude_home: &Path,
    groups: &[String],
) -> Result<Vec<String>, String> {
    let mut removed = Vec::new();
    for item in groups.iter().flat_map(|group| items(group)) {
        let link = account_dir.join(item);
        if is_our_link(&link, &claude_home.join(item)) {
            std::fs::remove_file(&link)
                .map_err(|error| format!("Could not unlink {} ({error}).", link.display()))?;
            removed.push(item.to_string());
        }
    }
    Ok(removed)
}

/// Copy the user-scope `mcpServers` of `default_json` into a new account's
/// `account_json`, when that file doesn't exist yet. Returns whether it
/// copied any.
pub fn seed_mcp_servers(default_json: &Path, account_json: &Path) -> Result<bool, String> {
    if std::fs::symlink_metadata(account_json).is_ok() {
        return Ok(false);
    }
    let Ok(text) = std::fs::read_to_string(default_json) else {
        return Ok(false);
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Ok(false);
    };
    let Some(servers) = value
        .get("mcpServers")
        .and_then(Value::as_object)
        .filter(|servers| !servers.is_empty())
    else {
        return Ok(false);
    };
    super::registry::write_atomic(account_json, &serde_json::json!({ "mcpServers": servers }))?;
    Ok(true)
}

/// Delete an account's folder: only a real folder directly inside `root`,
/// and never what its links point at.
pub fn delete_account_dir(root: &Path, dir: &Path) -> Result<(), String> {
    let inside = dir.parent() == Some(root)
        && dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| !name.is_empty() && name != "." && name != "..");
    if !inside {
        return Err(format!(
            "{} is not an account folder; it was not deleted.",
            dir.display()
        ));
    }
    let meta = match std::fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not look at {} ({error}).", dir.display())),
    };
    if !meta.is_dir() {
        return Err(format!(
            "{} is not a folder (a link?); it was not deleted.",
            dir.display()
        ));
    }
    let (Ok(real_root), Ok(real_dir)) = (std::fs::canonicalize(root), std::fs::canonicalize(dir))
    else {
        return Err(format!("Could not resolve {}.", dir.display()));
    };
    if real_dir.parent() != Some(real_root.as_path()) {
        return Err(format!(
            "{} is not inside the accounts folder; it was not deleted.",
            dir.display()
        ));
    }
    // remove_dir_all unlinks symlinks without following them.
    std::fs::remove_dir_all(dir)
        .map_err(|error| format!("Could not delete {} ({error}).", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Dirs {
        base: PathBuf,
        home: PathBuf,
        account: PathBuf,
    }

    impl Dirs {
        fn new(name: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "tmux-shared-{name}-{}",
                uuid::Uuid::new_v4().simple()
            ));
            let home = base.join(".claude");
            let account = base.join("accounts/personal");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(&account).unwrap();
            Self {
                base,
                home,
                account,
            }
        }
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    fn all() -> Vec<String> {
        GROUPS.iter().map(|g| g.to_string()).collect()
    }

    #[test]
    fn groups_are_checked_and_ordered() {
        assert_eq!(
            validate_groups(&["plugins".into(), "settings".into(), "plugins".into()]).unwrap(),
            vec!["settings", "plugins"]
        );
        assert!(validate_groups(&["credentials".into()]).is_err());
        for group in GROUPS {
            for item in items(group) {
                assert!(!NEVER.contains(item), "{item} is never shared");
            }
        }
    }

    #[test]
    fn links_are_made_once_and_never_replace_a_real_file() {
        let dirs = Dirs::new("link");
        for folder in ["skills", "projects", "plugins"] {
            std::fs::create_dir_all(dirs.home.join(folder)).unwrap();
        }
        std::fs::write(dirs.home.join("settings.json"), "{}").unwrap();
        std::fs::write(dirs.home.join("CLAUDE.md"), "# rules").unwrap();
        std::fs::write(dirs.home.join("history.jsonl"), "").unwrap();
        std::fs::write(dirs.home.join(".credentials.json"), "secret").unwrap();
        // The account already has its own CLAUDE.md.
        std::fs::write(dirs.account.join("CLAUDE.md"), "# mine").unwrap();
        let linked = link(&dirs.account, &dirs.home, &all()).unwrap();
        assert_eq!(
            linked.made,
            vec![
                "settings.json",
                "skills",
                "plugins",
                "projects",
                "history.jsonl"
            ]
        );
        assert_eq!(linked.own, vec!["CLAUDE.md"]);
        assert!(linked.missing.contains(&"keybindings.json".to_string()));
        assert_eq!(
            std::fs::read_to_string(dirs.account.join("CLAUDE.md")).unwrap(),
            "# mine"
        );
        assert_eq!(
            std::fs::read_link(dirs.account.join("projects")).unwrap(),
            dirs.home.join("projects")
        );
        assert!(!dirs.account.join(".credentials.json").exists());
        assert!(!dirs.account.join("sessions").exists());
        // Writing through a link writes the default account's file.
        std::fs::write(dirs.account.join("skills/x.md"), "x").unwrap();
        assert!(dirs.home.join("skills/x.md").exists());
        // Again: nothing new, an item that appeared since gets linked.
        std::fs::create_dir_all(dirs.home.join("todos")).unwrap();
        let again = link(&dirs.account, &dirs.home, &all()).unwrap();
        assert_eq!(again.made, vec!["todos"]);
        assert_eq!(again.linked.len(), 5);
    }

    #[test]
    fn unlinking_removes_only_our_links() {
        let dirs = Dirs::new("unlink");
        std::fs::create_dir_all(dirs.home.join("projects")).unwrap();
        std::fs::create_dir_all(dirs.home.join("projects/p")).unwrap();
        std::fs::write(dirs.home.join("projects/p/s.jsonl"), "{}").unwrap();
        std::fs::create_dir_all(dirs.home.join("skills")).unwrap();
        link(&dirs.account, &dirs.home, &all()).unwrap();
        // Someone else's link and a real file stay.
        std::fs::remove_file(dirs.account.join("skills")).unwrap();
        std::os::unix::fs::symlink("/elsewhere", dirs.account.join("skills")).unwrap();
        std::fs::write(dirs.account.join("history.jsonl"), "").unwrap();
        let removed = unlink(&dirs.account, &dirs.home, &all()).unwrap();
        assert_eq!(removed, vec!["projects"]);
        assert!(std::fs::symlink_metadata(dirs.account.join("skills")).is_ok());
        assert!(dirs.account.join("history.jsonl").exists());
        assert!(
            dirs.home.join("projects/p/s.jsonl").exists(),
            "targets kept"
        );
    }

    #[test]
    fn mcp_servers_are_copied_once() {
        let dirs = Dirs::new("mcp");
        let default_json = dirs.base.join(".claude.json");
        let account_json = dirs.account.join(".claude.json");
        assert!(!seed_mcp_servers(&default_json, &account_json).unwrap());
        std::fs::write(
            &default_json,
            r#"{"oauthAccount":{"emailAddress":"you@company.com"},"mcpServers":{"docs":{"command":"docs-mcp"}}}"#,
        )
        .unwrap();
        assert!(seed_mcp_servers(&default_json, &account_json).unwrap());
        let copied: Value =
            serde_json::from_str(&std::fs::read_to_string(&account_json).unwrap()).unwrap();
        assert_eq!(copied["mcpServers"]["docs"]["command"], "docs-mcp");
        assert!(copied.get("oauthAccount").is_none(), "only the servers");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&account_json)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::write(&account_json, "{}").unwrap();
        assert!(!seed_mcp_servers(&default_json, &account_json).unwrap());
        assert_eq!(std::fs::read_to_string(&account_json).unwrap(), "{}");
    }

    #[test]
    fn deleting_stays_inside_the_root_and_keeps_link_targets() {
        let dirs = Dirs::new("delete");
        let root = dirs.base.join("accounts");
        std::fs::create_dir_all(dirs.home.join("projects")).unwrap();
        std::fs::write(dirs.home.join("projects/keep.jsonl"), "{}").unwrap();
        link(&dirs.account, &dirs.home, &all()).unwrap();
        std::fs::write(dirs.account.join(".credentials.json"), "x").unwrap();
        assert!(delete_account_dir(&root, &dirs.base.join("elsewhere/personal")).is_err());
        assert!(delete_account_dir(&root, &root.join("../.claude")).is_err());
        // A link in the root that points out is never followed.
        std::os::unix::fs::symlink(&dirs.home, root.join("sneaky")).unwrap();
        assert!(delete_account_dir(&root, &root.join("sneaky")).is_err());
        assert!(dirs.home.join("projects/keep.jsonl").exists());
        delete_account_dir(&root, &dirs.account).unwrap();
        assert!(!dirs.account.exists());
        assert!(dirs.home.join("projects/keep.jsonl").exists());
        delete_account_dir(&root, &dirs.account).unwrap();
    }
}
