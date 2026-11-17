//! Adding, changing and removing accounts, and what signs one in
//! (agent-accounts §3.4, D1, D2, D17, G2, G10).

use super::launch::Target;
use super::paths::ClaudePaths;
use super::registry::{self, Entry, Registry};
use super::slot::Place;
use super::{Accounts, DEFAULT, shared};
use crate::model::now_ms;

/// An account a swap involves is not removed until the swap is over.
pub const SWAP_UNDER_WAY: &str =
    "A switch of the global account involving it is under way; try again once it is over.";

/// `remove` with `delete_files` while the account signs in.
pub const SIGNING_IN_THERE: &str = "A sign-in runs on it: cancel it (tmux::accounts::login_cancel) and let Claude Code stop, or remove the account and keep its folder.";

pub fn no_account(id: &str) -> String {
    format!("There is no Claude Code account \"{id}\".")
}

fn name_taken(registry: &Registry, name: &str, except: Option<&str>) -> bool {
    registry
        .accounts
        .iter()
        .filter(|entry| Some(entry.id.as_str()) != except)
        .any(|entry| entry.name.to_lowercase() == name.to_lowercase())
}

impl Accounts {
    fn paths_or_err(&self) -> Result<ClaudePaths, String> {
        self.paths.clone().ok_or_else(|| {
            "This computer's user has no home folder for Claude Code accounts.".into()
        })
    }

    /// A new account: its folder (`0700`) under the accounts root, the links
    /// of `shared` (every group when `None`), the MCP servers copied with
    /// `plugins`, and its entry. Returns its id.
    pub fn add(
        &self,
        name: &str,
        color: &str,
        shared: Option<Vec<String>>,
    ) -> Result<String, String> {
        let paths = self.paths_or_err()?;
        let name = registry::validate_name(name)?;
        let color = registry::validate_color(color)?;
        let groups = match shared {
            Some(groups) => shared::validate_groups(&groups)?,
            None => shared::GROUPS.iter().map(|g| g.to_string()).collect(),
        };
        self.update_registry(|registry| {
            if name_taken(registry, &name, None) {
                return Err(format!("There is already an account named \"{name}\"."));
            }
            let id = registry.new_id(&name, |id| {
                std::fs::symlink_metadata(paths.account_dir(id)).is_ok()
            });
            let dir = paths.account_dir(&id);
            registry::make_private_dir(&paths.root)?;
            {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&dir)
                    .map_err(|error| format!("Could not make {} ({error}).", dir.display()))?;
            }
            if let Err(error) = shared::link(&dir, &paths.claude_home, &groups) {
                let _ = shared::delete_account_dir(&paths.root, &dir);
                return Err(error);
            }
            if groups.iter().any(|g| g == "plugins")
                && let Err(error) =
                    shared::seed_mcp_servers(&paths.default_json, &paths.account_json(&id))
            {
                tracing::warn!(account = %id, %error, "MCP servers not copied");
            }
            registry.accounts.push(Entry {
                id: id.clone(),
                name,
                color,
                shared: groups,
                created_at_ms: now_ms(),
            });
            Ok(id)
        })
    }

    /// Rename, recolour or change what account `id` shares (links made for
    /// the groups added, the worker's links removed for those dropped).
    pub fn update(
        &self,
        id: &str,
        name: Option<&str>,
        color: Option<&str>,
        shared: Option<Vec<String>>,
    ) -> Result<(), String> {
        let paths = self.paths_or_err()?;
        let name = name.map(registry::validate_name).transpose()?;
        let color = color.map(registry::validate_color).transpose()?;
        let groups = shared
            .map(|groups| shared::validate_groups(&groups))
            .transpose()?;
        self.update_registry(|registry| {
            if !registry.has(id) {
                return Err(no_account(id));
            }
            if let Some(name) = name.as_deref()
                && name_taken(registry, name, Some(id))
            {
                return Err(format!("There is already an account named \"{name}\"."));
            }
            let entry = registry.get_mut(id).expect("checked above");
            if let Some(groups) = groups {
                let dir = paths.account_dir(id);
                let dropped: Vec<String> = entry
                    .shared
                    .iter()
                    .filter(|g| !groups.contains(g))
                    .cloned()
                    .collect();
                shared::unlink(&dir, &paths.claude_home, &dropped)?;
                if dir.is_dir() {
                    shared::link(&dir, &paths.claude_home, &groups)?;
                }
                entry.shared = groups;
            }
            if let Some(name) = name {
                entry.name = name;
            }
            if let Some(color) = color {
                entry.color = color;
            }
            Ok(())
        })
    }

    /// Take account `id` off the list. Never `default`, the global one, the
    /// one a switch waits to make global, nor one a swap running or left
    /// half-way involves ([`SWAP_UNDER_WAY`]).
    /// Its panes keep running; a sign-in of it from the app stops. With
    /// `delete_files` its folder goes too (only inside the accounts root,
    /// never what its links point at), refused while `running` Claude Code
    /// panes still run on it, or a sign-in does.
    pub fn remove(&self, id: &str, delete_files: bool, running: u32) -> Result<(), String> {
        let paths = self.paths_or_err()?;
        if id == DEFAULT {
            return Err("The default account can't be removed.".into());
        }
        // A swap that involves it, running or left half-way (its journal):
        // its sign-in may be on its way into or out of its folder.
        if self
            .stores()
            .and_then(|stores| stores.read_journal())
            .is_some_and(|journal| journal.from == id || journal.to == id)
        {
            return Err(SWAP_UNDER_WAY.into());
        }
        self.update_registry(|registry| {
            if !registry.has(id) {
                return Err(no_account(id));
            }
            if registry.global == id {
                return Err("It is the global account: make another account global first.".into());
            }
            if registry
                .pending_global
                .as_ref()
                .is_some_and(|pending| pending.account == id)
            {
                return Err("A switch waits to make it global: cancel it first.".into());
            }
            if delete_files && running > 0 {
                let (panes, run, them) = if running == 1 {
                    ("pane", "runs", "it")
                } else {
                    ("panes", "run", "them")
                };
                return Err(format!(
                    "{running} Claude Code {panes} still {run} on it: move or close {them} first, or remove the account and keep its folder."
                ));
            }
            // Claude Code would save that sign-in in the folder deleted.
            if delete_files && self.logins.signing_in(id) {
                return Err(SIGNING_IN_THERE.into());
            }
            if delete_files {
                shared::delete_account_dir(&paths.root, &paths.account_dir(id))?;
            }
            registry.accounts.retain(|entry| entry.id != id);
            registry.dismissed.retain(|dismissal| dismissal.from != id);
            Ok(())
        })?;
        self.forget(id);
        // A sign-in of the account removed has nowhere to land.
        self.logins.cancel_account(id);
        Ok(())
    }

    /// Where account `id` signs in (G10) and the email its sign-in page is
    /// filled with: on the slot for the global account (a sign-in to
    /// another account there is taken in, G6), on its folder otherwise.
    /// Makes its folder again if it went missing, and checks its links.
    pub fn login_plan(
        &self,
        id: &str,
        browser: crate::model::LoginBrowser,
    ) -> Result<super::login::Plan, String> {
        let paths = self.paths_or_err()?;
        let registry = self.registry();
        let entry = registry.get(id).ok_or_else(|| no_account(id))?;
        let dir = paths.account_dir(id);
        registry::make_private_dir(&dir)?;
        if let Err(error) = shared::link(&dir, &paths.claude_home, &entry.shared) {
            tracing::warn!(account = %id, %error, "shared items not linked");
        }
        self.reread_credentials(id);
        let config_dir = match self.login_target(id)? {
            Target::Dir(dir) => Some(std::path::PathBuf::from(dir)),
            Target::Slot | Target::Plain => None,
        };
        let before = self.place_mark(&if config_dir.is_some() {
            Place::Folder(id.to_string())
        } else {
            Place::Slot
        });
        Ok(super::login::Plan {
            account: id.to_string(),
            config_dir,
            home: paths.home.clone(),
            email: self.email_of(id),
            browser,
            before,
        })
    }

    /// "Not now" on suggestion `id`: no suggestion from that account for
    /// that reason (or a lower one) until its window resets (D14).
    pub fn dismiss(&self, id: &str) -> Result<(), String> {
        let now = now_ms();
        let accounts = self.snapshot(&super::PaneCounts::default()).claude.accounts;
        let dismissal = super::rules::dismissal(id, &accounts, now)?;
        self.update_registry(|registry| {
            registry
                .dismissed
                .retain(|d| d.until_ms > now && d.from != dismissal.from);
            registry.dismissed.push(dismissal);
            Ok(())
        })
    }

    /// Change the rules; what `change` leaves out stays.
    pub fn set_rules(
        &self,
        change: impl FnOnce(&mut crate::model::SwitchRules),
    ) -> Result<crate::model::SwitchRules, String> {
        self.update_registry(|registry| {
            change(&mut registry.rules);
            if let Some(unknown) = registry.rules.order.iter().find(|id| !registry.has(id)) {
                return Err(no_account(unknown));
            }
            Ok(())
        })?;
        Ok(self.registry().rules)
    }

    /// Link what each account shares and `~/.claude` has gained since
    /// (worker start).
    pub fn relink_all(&self) {
        let Some(paths) = self.paths.clone() else {
            return;
        };
        if !self.managed() {
            return;
        }
        for entry in self.registry().accounts {
            let dir = paths.account_dir(&entry.id);
            if entry.shared.is_empty() || !dir.is_dir() {
                continue;
            }
            match shared::link(&dir, &paths.claude_home, &entry.shared) {
                Ok(linked) if !linked.made.is_empty() => {
                    tracing::info!(account = %entry.id, made = ?linked.made, "shared items linked");
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(account = %entry.id, %error, "shared items not linked")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::Home;
    use super::*;

    #[test]
    fn adding_makes_a_private_folder_with_links() {
        let home = Home::new("add");
        let claude = &home.paths.claude_home;
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        std::fs::write(claude.join("settings.json"), "{}").unwrap();
        std::fs::write(
            &home.paths.default_json,
            r#"{"mcpServers":{"docs":{"command":"docs-mcp"}}}"#,
        )
        .unwrap();
        let accounts = home.accounts();
        let id = accounts.add(" Side project ", "violet", None).unwrap();
        assert_eq!(id, "side-project");
        let dir = home.paths.account_dir(&id);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::read_link(dir.join("projects")).unwrap(),
            claude.join("projects")
        );
        assert!(
            std::fs::read_to_string(dir.join(".claude.json"))
                .unwrap()
                .contains("docs-mcp")
        );
        let registry = accounts.registry();
        assert!(accounts.managed());
        let entry = registry.get(&id).unwrap();
        assert_eq!(entry.name, "Side project");
        assert_eq!(entry.shared.len(), 4);
        assert!(entry.created_at_ms > 0);
        assert!(accounts.add("side project", "green", None).is_err());
        assert!(accounts.add("Other", "red", None).is_err());
        assert!(
            accounts
                .add("Other", "green", Some(vec!["keys".into()]))
                .is_err()
        );
        // Nothing shared: no links, no servers copied.
        let bare = accounts.add("Bare", "orange", Some(vec![])).unwrap();
        let bare_dir = home.paths.account_dir(&bare);
        assert!(std::fs::read_dir(&bare_dir).unwrap().next().is_none());
    }

    #[test]
    fn a_folder_left_behind_is_never_reused() {
        let home = Home::new("leftover");
        std::fs::create_dir_all(home.paths.root.join("personal")).unwrap();
        let accounts = home.accounts();
        assert_eq!(
            accounts.add("Personal", "green", Some(vec![])).unwrap(),
            "personal-2"
        );
    }

    #[test]
    fn updating_changes_links_and_names() {
        let home = Home::new("update");
        let claude = &home.paths.claude_home;
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        std::fs::create_dir_all(claude.join("skills")).unwrap();
        let accounts = home.accounts();
        let id = accounts.add("Personal", "green", None).unwrap();
        let dir = home.paths.account_dir(&id);
        accounts
            .update(
                &id,
                Some("Home"),
                Some("pink"),
                Some(vec!["extensions".into()]),
            )
            .unwrap();
        assert!(std::fs::symlink_metadata(dir.join("projects")).is_err());
        assert!(std::fs::symlink_metadata(dir.join("skills")).is_ok());
        let registry = accounts.registry();
        let entry = registry.get(&id).unwrap();
        assert_eq!(
            (entry.name.as_str(), entry.color.as_str()),
            ("Home", "pink")
        );
        assert_eq!(entry.shared, vec!["extensions"]);
        accounts
            .update("default", Some("Work"), Some("blue"), None)
            .unwrap();
        assert_eq!(accounts.registry().accounts[0].name, "Work");
        // default has a folder of its own now (G2): what it shares changes.
        accounts
            .update("default", None, None, Some(vec!["settings".into()]))
            .unwrap();
        assert_eq!(accounts.registry().accounts[0].shared, vec!["settings"]);
        assert!(
            accounts
                .update("default", Some("home"), None, None)
                .is_err()
        );
        assert!(accounts.update("nobody", Some("x"), None, None).is_err());
    }

    #[test]
    fn removing_refuses_default_the_global_and_busy_folders() {
        let home = Home::new("remove");
        std::fs::create_dir_all(home.paths.claude_home.join("projects")).unwrap();
        std::fs::write(home.paths.claude_home.join("projects/keep"), "").unwrap();
        let accounts = home.accounts();
        let id = accounts.add("Personal", "green", None).unwrap();
        let dir = home.paths.account_dir(&id);
        assert!(accounts.remove("default", false, 0).is_err());
        accounts
            .update_registry(|registry| {
                registry.global = id.clone();
                Ok(())
            })
            .unwrap();
        assert!(accounts.remove(&id, false, 0).is_err(), "the global one");
        accounts
            .update_registry(|registry| {
                registry.global = DEFAULT.into();
                Ok(())
            })
            .unwrap();
        assert!(accounts.remove(&id, true, 2).is_err(), "panes run on it");
        assert!(dir.is_dir());
        accounts.remove(&id, true, 0).unwrap();
        assert!(!dir.exists());
        assert!(home.paths.claude_home.join("projects/keep").exists());
        assert!(!accounts.registry().has(&id));
        assert!(accounts.remove(&id, false, 0).is_err());
        // Without delete_files the folder stays.
        let other = accounts.add("Other", "pink", None).unwrap();
        accounts.remove(&other, false, 3).unwrap();
        assert!(home.paths.account_dir(&other).is_dir());
    }

    #[test]
    fn logins_run_on_the_slot_or_the_account_folder() {
        use crate::model::LoginBrowser;
        let home = Home::new("login");
        let accounts = home.accounts();
        // The global account signs in on the slot (G10).
        let plan = accounts.login_plan("default", LoginBrowser::None).unwrap();
        assert_eq!(plan.config_dir, None);
        assert_eq!(plan.home, home.paths.home);
        assert_eq!(plan.account, "default");
        let id = accounts.add("Personal", "green", Some(vec![])).unwrap();
        let dir = home.paths.account_dir(&id);
        std::fs::remove_dir(&dir).unwrap();
        let plan = accounts.login_plan(&id, LoginBrowser::Host).unwrap();
        assert_eq!(plan.config_dir.as_deref(), Some(dir.as_path()));
        assert_eq!(plan.browser, LoginBrowser::Host);
        assert_eq!(plan.email, None, "signed out: no email yet");
        assert!(dir.is_dir(), "made again");
        assert!(accounts.login_plan("nobody", LoginBrowser::None).is_err());
    }

    #[test]
    fn relinking_picks_up_new_items() {
        let home = Home::new("relink");
        let accounts = home.accounts();
        let id = accounts.add("Personal", "green", None).unwrap();
        let dir = home.paths.account_dir(&id);
        assert!(std::fs::symlink_metadata(dir.join("plans")).is_err());
        std::fs::create_dir_all(home.paths.claude_home.join("plans")).unwrap();
        accounts.relink_all();
        assert!(std::fs::symlink_metadata(dir.join("plans")).is_ok());
    }
}
