//! A Claude Code account's sign-in and profile, read-only (agent-accounts
//! D10, D11).
//!
//! On Linux the sign-in is `<config>/.credentials.json`. On macOS it is in
//! the login Keychain, as a generic password whose service is
//! `Claude Code-credentials` for the default account and
//! `Claude Code-credentials-<first 8 hex of sha256(CLAUDE_CONFIG_DIR)>` for
//! any other (Claude Code 2.1.322 hashes the variable's value as given, NFC
//! normalised), and whose account is `$USER`; the file is its fallback there.
//! The token is never logged, printed or sent anywhere but the usage endpoint.
//!
//! The profile (email, organization, plan) and the usage reading Claude Code
//! itself cached (`cachedUsageUtilization`) are in the account's
//! `.claude.json`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// A token. Its `Debug` never shows it, and it is never serialized.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The token, for the one header that carries it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// What Claude Code's sign-in says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    pub token: Option<Secret>,
    pub expires_at_ms: Option<u64>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

impl Credentials {
    pub fn signed_in(&self) -> bool {
        self.token.is_some()
    }

    /// The token expires within a minute (or did); it is not sent then.
    pub fn expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms
            .is_some_and(|expires| expires <= now_ms.saturating_add(60_000))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsFile {
    claude_ai_oauth: Option<OauthEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OauthEntry {
    access_token: Option<String>,
    expires_at: Option<u64>,
    subscription_type: Option<String>,
    rate_limit_tier: Option<String>,
}

/// `{"claudeAiOauth": {"accessToken", "expiresAt" (ms), "subscriptionType",
/// "rateLimitTier", …}}`, as in the file and the Keychain item.
pub fn parse_credentials(text: &str) -> Result<Credentials, String> {
    let file: CredentialsFile =
        serde_json::from_str(text).map_err(|_| "The sign-in can't be read.".to_string())?;
    let Some(oauth) = file.claude_ai_oauth else {
        return Ok(Credentials::default());
    };
    Ok(Credentials {
        token: oauth
            .access_token
            .filter(|token| !token.trim().is_empty())
            .map(Secret::new),
        expires_at_ms: oauth.expires_at,
        subscription_type: oauth.subscription_type.filter(|s| !s.is_empty()),
        rate_limit_tier: oauth.rate_limit_tier.filter(|s| !s.is_empty()),
    })
}

/// The Keychain service Claude Code keeps an account's sign-in under:
/// `config_dir_env` is the `CLAUDE_CONFIG_DIR` it runs with (`None` for the
/// default account).
pub fn keychain_service(config_dir_env: Option<&str>) -> String {
    match config_dir_env.filter(|dir| !dir.is_empty()) {
        None => "Claude Code-credentials".to_string(),
        Some(dir) => {
            let digest = Sha256::digest(dir.as_bytes());
            let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
            format!("Claude Code-credentials-{hex}")
        }
    }
}

/// The Keychain account Claude Code uses: `$USER`, or the login name, or
/// `claude-code-user` when that has other characters than `[A-Za-z0-9._-]`.
pub fn keychain_account(user: Option<&str>) -> String {
    match user {
        Some(user)
            if !user.is_empty()
                && user
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) =>
        {
            user.to_string()
        }
        _ => "claude-code-user".to_string(),
    }
}

/// Where a sign-in is read from; a trait so tests never touch a Keychain.
pub trait SecretStore: Send + Sync {
    /// The sign-in of the account in `dir`, `env_dir` being the
    /// `CLAUDE_CONFIG_DIR` it runs with (`None` for the default). `Ok(None)`
    /// when it has none. Blocking.
    fn read(&self, dir: &Path, env_dir: Option<&str>) -> Result<Option<Credentials>, String>;
}

/// `<dir>/.credentials.json`.
pub struct FileStore;

impl SecretStore for FileStore {
    fn read(&self, dir: &Path, _env_dir: Option<&str>) -> Result<Option<Credentials>, String> {
        read_file(&dir.join(".credentials.json"))
    }
}

fn read_file(path: &Path) -> Result<Option<Credentials>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_credentials(&text).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("The sign-in can't be read.".to_string()),
    }
}

/// macOS: the login Keychain through `security`, then the file.
pub struct KeychainStore {
    pub account: String,
}

/// How long `security` may take (it waits while the Keychain is locked).
const SECURITY_TIMEOUT: Duration = Duration::from_secs(5);
/// `security`'s exit code for an item that is not there.
const ITEM_NOT_FOUND: i32 = 44;

impl SecretStore for KeychainStore {
    fn read(&self, dir: &Path, env_dir: Option<&str>) -> Result<Option<Credentials>, String> {
        let service = keychain_service(env_dir);
        let mut child = std::process::Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-a",
                &self.account,
                "-w",
                "-s",
                &service,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| "The Keychain can't be read.".to_string())?;
        let started = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() > SECURITY_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("The Keychain did not answer.".to_string());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => return Err("The Keychain can't be read.".to_string()),
            }
        };
        let mut stdout = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            use std::io::Read;
            let _ = pipe.read_to_string(&mut stdout);
        }
        if status.success() {
            return parse_credentials(stdout.trim()).map(Some);
        }
        if status.code() == Some(ITEM_NOT_FOUND) {
            // Claude Code falls back to the file when the Keychain refuses.
            return read_file(&dir.join(".credentials.json"));
        }
        Err("The Keychain refused to give the sign-in.".to_string())
    }
}

/// This platform's store.
pub fn platform_store() -> Box<dyn SecretStore> {
    if cfg!(target_os = "macos") {
        Box::new(KeychainStore {
            account: keychain_account(std::env::var("USER").ok().as_deref()),
        })
    } else {
        Box::new(FileStore)
    }
}

/// What an account's `.claude.json` says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profile {
    pub email: Option<String>,
    pub organization: Option<String>,
    /// `organizationType` (`claude_max`, `claude_team`, …).
    pub organization_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    /// Claude Code's own last usage reading: when, and the response.
    pub cached_usage: Option<(u64, serde_json::Value)>,
    /// User-scope MCP servers (copied once into a new account, D17).
    pub mcp_servers: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileFile {
    oauth_account: Option<OauthAccount>,
    cached_usage_utilization: Option<CachedUsage>,
    mcp_servers: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OauthAccount {
    email_address: Option<String>,
    organization_name: Option<String>,
    organization_type: Option<String>,
    organization_rate_limit_tier: Option<String>,
    user_rate_limit_tier: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedUsage {
    fetched_at_ms: Option<u64>,
    utilization: Option<serde_json::Value>,
}

pub fn parse_profile(text: &str) -> Option<Profile> {
    let file: ProfileFile = serde_json::from_str(text).ok()?;
    let account = file.oauth_account;
    let nonempty = |value: Option<String>| value.filter(|v| !v.trim().is_empty());
    Some(Profile {
        email: nonempty(account.as_ref().and_then(|a| a.email_address.clone())),
        organization: nonempty(account.as_ref().and_then(|a| a.organization_name.clone())),
        organization_type: nonempty(account.as_ref().and_then(|a| a.organization_type.clone())),
        rate_limit_tier: account.as_ref().and_then(|a| {
            nonempty(a.user_rate_limit_tier.clone())
                .or_else(|| nonempty(a.organization_rate_limit_tier.clone()))
        }),
        cached_usage: file.cached_usage_utilization.and_then(|cached| {
            Some((
                cached.fetched_at_ms?,
                cached.utilization.filter(|v| v.is_object())?,
            ))
        }),
        mcp_servers: file
            .mcp_servers
            .filter(|servers| servers.as_object().is_some_and(|o| !o.is_empty())),
    })
}

/// The plan people know: the sign-in's subscription type, else the
/// organization's type without its `claude_` prefix.
pub fn plan(credentials: Option<&Credentials>, profile: Option<&Profile>) -> Option<String> {
    credentials
        .and_then(|c| c.subscription_type.clone())
        .or_else(|| {
            let kind = profile?.organization_type.as_deref()?;
            Some(kind.strip_prefix("claude_").unwrap_or(kind).to_string())
        })
}

/// A file's modification time and size, if it exists.
type Stamp = Option<(SystemTime, u64)>;

/// A file read again only when its size or time changes.
#[derive(Debug, Default)]
pub struct FileCache<T> {
    entries: std::collections::HashMap<PathBuf, (Stamp, Option<T>)>,
}

impl<T: Clone> FileCache<T> {
    pub fn get(&mut self, path: &Path, parse: impl FnOnce(&str) -> Option<T>) -> Option<T> {
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
        if let Some((seen, value)) = self.entries.get(path) {
            if *seen == stamp {
                return value.clone();
            }
        }
        let value = stamp
            .and_then(|_| std::fs::read_to_string(path).ok())
            .and_then(|text| parse(&text));
        self.entries
            .insert(path.to_path_buf(), (stamp, value.clone()));
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{"claudeAiOauth":{"accessToken":"demo-work","refreshToken":"r","expiresAt":1794443325000,
        "scopes":["user:inference"],"subscriptionType":"team","rateLimitTier":"default_claude_team"},
        "mcpOAuth":{"x":{"accessToken":"other"}}}"#;

    #[test]
    fn credentials_parse_and_never_show_the_token() {
        let credentials = parse_credentials(FILE).unwrap();
        assert!(credentials.signed_in());
        assert_eq!(credentials.expires_at_ms, Some(1_794_443_325_000));
        assert_eq!(credentials.subscription_type.as_deref(), Some("team"));
        assert_eq!(
            credentials.rate_limit_tier.as_deref(),
            Some("default_claude_team")
        );
        let shown = format!("{credentials:?}");
        assert!(!shown.contains("demo-work"), "{shown}");
        assert!(credentials.expired(1_794_443_325_000 - 30_000));
        assert!(!credentials.expired(1_794_443_325_000 - 120_000));
        assert!(!parse_credentials(r#"{"mcpOAuth":{}}"#).unwrap().signed_in());
        assert!(
            !parse_credentials(r#"{"claudeAiOauth":{"accessToken":" "}}"#)
                .unwrap()
                .signed_in()
        );
        let error = parse_credentials("{oops demo-work").unwrap_err();
        assert!(!error.contains("demo-work"));
    }

    #[test]
    fn keychain_names_follow_claude_code() {
        assert_eq!(keychain_service(None), "Claude Code-credentials");
        assert_eq!(keychain_service(Some("")), "Claude Code-credentials");
        // sha256("/Users/me/.local/share/iii/claude-accounts/personal")
        let expected = {
            let digest = Sha256::digest(b"/Users/me/.local/share/iii/claude-accounts/personal");
            format!(
                "Claude Code-credentials-{:02x}{:02x}{:02x}{:02x}",
                digest[0], digest[1], digest[2], digest[3]
            )
        };
        assert_eq!(
            keychain_service(Some("/Users/me/.local/share/iii/claude-accounts/personal")),
            expected
        );
        assert_eq!(
            keychain_service(Some("/tmp/x")).len(),
            "Claude Code-credentials-".len() + 8
        );
        assert_eq!(keychain_account(Some("sergio")), "sergio");
        assert_eq!(keychain_account(Some("José")), "claude-code-user");
        assert_eq!(keychain_account(None), "claude-code-user");
    }

    #[test]
    fn the_file_store_reads_the_folder() {
        let dir =
            std::env::temp_dir().join(format!("tmux-creds-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(FileStore.read(&dir, None).unwrap(), None);
        std::fs::write(dir.join(".credentials.json"), FILE).unwrap();
        assert!(FileStore.read(&dir, None).unwrap().unwrap().signed_in());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn profiles_read_email_plan_and_cached_usage() {
        let text = r#"{"numStartups":3,"oauthAccount":{"emailAddress":"you@company.com","organizationName":"Company",
            "organizationType":"claude_team","organizationRateLimitTier":"default_claude_team","userRateLimitTier":null},
            "cachedUsageUtilization":{"fetchedAtMs":1794421293312,"utilization":{"seven_day":{"utilization":73,"resets_at":"2026-11-17T19:00:00+00:00"}}},
            "mcpServers":{"fs":{"command":"x"}}}"#;
        let profile = parse_profile(text).unwrap();
        assert_eq!(profile.email.as_deref(), Some("you@company.com"));
        assert_eq!(profile.organization.as_deref(), Some("Company"));
        assert_eq!(
            profile.rate_limit_tier.as_deref(),
            Some("default_claude_team")
        );
        assert_eq!(profile.cached_usage.as_ref().unwrap().0, 1_794_421_293_312);
        assert!(profile.mcp_servers.is_some());
        assert_eq!(plan(None, Some(&profile)).as_deref(), Some("team"));
        let credentials = parse_credentials(FILE).unwrap();
        assert_eq!(plan(Some(&credentials), None).as_deref(), Some("team"));
        assert_eq!(parse_profile("{}").unwrap(), Profile::default());
    }
}
