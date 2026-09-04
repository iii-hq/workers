use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{bool_at, register_fn, run_json, run_text, spec, string_at, EmptyInput, FunctionSpec};
use crate::config::{SharedConfig, WorkerConfig};

pub const LOCK_STATUS_ID: &str = "tailscale::lock::status";
pub const ACCOUNTS_LIST_ID: &str = "tailscale::accounts::list";
pub const ACCOUNTS_SWITCH_ID: &str = "tailscale::accounts::switch";
pub const UPDATE_ID: &str = "tailscale::update";
pub const BUGREPORT_ID: &str = "tailscale::bugreport";
pub const METRICS_ID: &str = "tailscale::metrics";

const LOCK_STATUS_DESC: &str =
    "Report whether tailnet lock is enabled and this node's tailnet-lock public key.";
const ACCOUNTS_LIST_DESC: &str =
    "List the Tailscale accounts logged in on this device and which one is active.";
const ACCOUNTS_SWITCH_DESC: &str =
    "Switch this device to another logged-in Tailscale account by id, tailnet, or login name.";
const UPDATE_DESC: &str = "Update the Tailscale client to the latest release. With dry_run=true, only report what would change.";
const BUGREPORT_DESC: &str = "Generate a Tailscale bug report identifier that support can look up; optional note and in-depth diagnosis.";
const METRICS_DESC: &str = "Return the client's user-facing metrics in Prometheus text format.";

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<EmptyInput, LockStatusOutput>(LOCK_STATUS_ID, LOCK_STATUS_DESC),
        spec::<EmptyInput, AccountsOutput>(ACCOUNTS_LIST_ID, ACCOUNTS_LIST_DESC),
        spec::<AccountSwitchInput, AccountsOutput>(ACCOUNTS_SWITCH_ID, ACCOUNTS_SWITCH_DESC),
        spec::<UpdateInput, TextOutput>(UPDATE_ID, UPDATE_DESC),
        spec::<BugreportInput, BugreportOutput>(BUGREPORT_ID, BUGREPORT_DESC),
        spec::<EmptyInput, TextOutput>(METRICS_ID, METRICS_DESC),
    ]
}

pub fn register(iii: &IIIClient, config: &SharedConfig) {
    register_fn!(
        iii,
        config,
        LOCK_STATUS_ID,
        LOCK_STATUS_DESC,
        EmptyInput,
        lock_status
    );
    register_fn!(
        iii,
        config,
        ACCOUNTS_LIST_ID,
        ACCOUNTS_LIST_DESC,
        EmptyInput,
        accounts_list
    );
    register_fn!(
        iii,
        config,
        ACCOUNTS_SWITCH_ID,
        ACCOUNTS_SWITCH_DESC,
        AccountSwitchInput,
        accounts_switch
    );
    register_fn!(iii, config, UPDATE_ID, UPDATE_DESC, UpdateInput, update);
    register_fn!(
        iii,
        config,
        BUGREPORT_ID,
        BUGREPORT_DESC,
        BugreportInput,
        bugreport
    );
    register_fn!(iii, config, METRICS_ID, METRICS_DESC, EmptyInput, metrics);
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LockStatusOutput {
    /// Whether tailnet lock is enabled for the tailnet.
    pub enabled: bool,
    /// This node's tailnet-lock public key (`tlpub:…`), safe to share with admins.
    pub node_key: Option<String>,
    /// Whether this node is signed under tailnet lock, when enabled.
    pub node_signed: Option<bool>,
    /// The CLI's own output.
    pub output: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Account {
    /// Short id used by `switch`.
    pub id: String,
    /// Login name.
    pub account: String,
    /// Tailnet name.
    pub tailnet: String,
    /// Nickname, when set.
    pub nickname: Option<String>,
    /// Whether this account is active.
    pub selected: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AccountsOutput {
    /// Accounts logged in on this device.
    pub accounts: Vec<Account>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AccountSwitchInput {
    /// Account id, tailnet, login name, or nickname from accounts::list.
    pub account: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct UpdateInput {
    /// Report what an update would do without applying it.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TextOutput {
    /// The CLI's own output.
    pub output: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct BugreportInput {
    /// Free-text note attached to the report.
    pub note: Option<String>,
    /// Run additional in-depth checks.
    #[serde(default)]
    pub diagnose: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BugreportOutput {
    /// Shareable bug report identifier.
    pub report_id: String,
    /// The CLI's own output.
    pub output: String,
}

pub fn parse_lock_status(text: &str) -> (bool, Option<String>, Option<bool>) {
    let lower = text.to_ascii_lowercase();
    let enabled = lower.contains("tailnet lock is enabled");
    let node_key = text
        .split_whitespace()
        .find(|word| word.starts_with("tlpub:"))
        .map(|word| word.trim_end_matches('.').to_string());
    let node_signed = if enabled {
        Some(!lower.contains("not signed"))
    } else {
        None
    };
    (enabled, node_key, node_signed)
}

async fn lock_status(config: &WorkerConfig, _: EmptyInput) -> Result<LockStatusOutput, String> {
    let output = run_text(config, &["lock", "status"]).await?;
    let (enabled, node_key, node_signed) = parse_lock_status(&output);
    Ok(LockStatusOutput {
        enabled,
        node_key,
        node_signed,
        output,
    })
}

pub fn parse_accounts(value: &Value) -> Vec<Account> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| Account {
            id: string_at(entry, "/id").unwrap_or_default(),
            account: string_at(entry, "/account").unwrap_or_default(),
            tailnet: string_at(entry, "/tailnet").unwrap_or_default(),
            nickname: string_at(entry, "/nickname").filter(|n| !n.is_empty()),
            selected: bool_at(entry, "/selected"),
        })
        .collect()
}

async fn accounts_list(config: &WorkerConfig, _: EmptyInput) -> Result<AccountsOutput, String> {
    let value = run_json(config, &["switch", "--list", "--json"]).await?;
    Ok(AccountsOutput {
        accounts: parse_accounts(&value),
    })
}

async fn accounts_switch(
    config: &WorkerConfig,
    input: AccountSwitchInput,
) -> Result<AccountsOutput, String> {
    let account = input.account.trim().to_string();
    if account.is_empty() || account.starts_with('-') || account.chars().any(char::is_whitespace) {
        return Err("account must be an id, tailnet, login name, or nickname".to_string());
    }
    run_text(config, &["switch", &account]).await?;
    accounts_list(config, EmptyInput::default()).await
}

async fn update(config: &WorkerConfig, input: UpdateInput) -> Result<TextOutput, String> {
    let output = if input.dry_run {
        run_text(config, &["update", "--dry-run"]).await?
    } else {
        run_text(config, &["update", "--yes"]).await?
    };
    Ok(TextOutput { output })
}

async fn bugreport(
    config: &WorkerConfig,
    input: BugreportInput,
) -> Result<BugreportOutput, String> {
    let mut args = vec!["bugreport".to_string()];
    if input.diagnose {
        args.push("--diagnose".to_string());
    }
    if let Some(note) = input
        .note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        if note.starts_with('-') {
            return Err("note must not start with -".to_string());
        }
        args.push(note.to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = run_text(config, &refs).await?;
    let report_id = output
        .split_whitespace()
        .find(|word| word.starts_with("BUG-"))
        .unwrap_or_default()
        .to_string();
    Ok(BugreportOutput { report_id, output })
}

async fn metrics(config: &WorkerConfig, _: EmptyInput) -> Result<TextOutput, String> {
    let output = run_text(config, &["metrics", "print"]).await?;
    Ok(TextOutput { output })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_status_is_parsed() {
        let (enabled, key, signed) = parse_lock_status(
            "Tailnet Lock is NOT enabled.\n\nThis node's tailnet-lock key: tlpub:abc123\n",
        );
        assert!(!enabled);
        assert_eq!(key.as_deref(), Some("tlpub:abc123"));
        assert!(signed.is_none());
        let (enabled, _, signed) = parse_lock_status(
            "Tailnet Lock is ENABLED.\nThis node is not signed by a trusted key.",
        );
        assert!(enabled);
        assert_eq!(signed, Some(false));
    }

    #[test]
    fn accounts_are_parsed() {
        let value = serde_json::json!([{"id": "d5a5", "nickname": "", "tailnet": "me@example.com", "account": "me@example.com", "selected": true}]);
        let accounts = parse_accounts(&value);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "d5a5");
        assert!(accounts[0].nickname.is_none());
        assert!(accounts[0].selected);
    }
}
