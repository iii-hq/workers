//! A Claude Code account's sign-in and profile (agent-accounts D10, D11):
//! read for the usage and the snapshot, and written only by a swap of the
//! global account (G3), which moves a sign-in between stores and never
//! changes a token.
//!
//! On Linux the sign-in is `<config>/.credentials.json`. On macOS it is in
//! the login Keychain, as a generic password whose service is
//! `Claude Code-credentials` for the default account and
//! `Claude Code-credentials-<first 8 hex of sha256(CLAUDE_CONFIG_DIR)>` for
//! any other (Claude Code 2.1.322 hashes the variable's value NFC-normalised:
//! `createHash("sha256").update(dir.normalize("NFC"))`), and whose account
//! is `$USER`; the file is its fallback there.
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
    /// The refresh token: while it is there, Claude Code renews the access
    /// token by itself (G9).
    pub refresh: Option<Secret>,
    pub expires_at_ms: Option<u64>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

impl Credentials {
    /// A sign-in Claude Code can use: an access token, or a refresh token
    /// that gets one (G9). A sign-in Claude Code cleared after a refusal
    /// (`invalid_grant`) has neither.
    pub fn signed_in(&self) -> bool {
        self.token.is_some() || self.refresh.is_some()
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
    refresh_token: Option<String>,
    expires_at: Option<u64>,
    subscription_type: Option<String>,
    rate_limit_tier: Option<String>,
}

/// `{"claudeAiOauth": {"accessToken", "expiresAt" (ms), "subscriptionType",
/// "rateLimitTier", …}}`, as in the file and the Keychain item.
pub fn parse_credentials(text: &str) -> Result<Credentials, String> {
    let file: CredentialsFile =
        serde_json::from_str(text).map_err(|_| "The sign-in can't be read.".to_string())?;
    Ok(from_file(file))
}

/// [`parse_credentials`] of a store already read.
pub fn credentials_of(raw: &Raw) -> Credentials {
    serde_json::from_value::<CredentialsFile>(serde_json::Value::Object(raw.clone()))
        .map(from_file)
        .unwrap_or_default()
}

fn from_file(file: CredentialsFile) -> Credentials {
    let Some(oauth) = file.claude_ai_oauth else {
        return Credentials::default();
    };
    Credentials {
        token: oauth
            .access_token
            .filter(|token| !token.trim().is_empty())
            .map(Secret::new),
        refresh: oauth
            .refresh_token
            .filter(|token| !token.trim().is_empty())
            .map(Secret::new),
        expires_at_ms: oauth.expires_at,
        subscription_type: oauth.subscription_type.filter(|s| !s.is_empty()),
        rate_limit_tier: oauth.rate_limit_tier.filter(|s| !s.is_empty()),
    }
}

/// A whole sign-in store: `claudeAiOauth` (the Claude sign-in) and what
/// else Claude Code keeps there (`mcpOAuth`, …).
pub type Raw = serde_json::Map<String, serde_json::Value>;

/// The key of the Claude sign-in in a store.
pub const OAUTH_KEY: &str = "claudeAiOauth";

/// What tells one sign-in in `raw` from the next without keeping a token:
/// the first 16 hex of the SHA-256 of its access token, refresh token and
/// expiry (a new sign-in, or a renewal, changes them). `None` without a
/// sign-in Claude Code can use.
pub fn sign_in_mark(raw: &Raw) -> Option<String> {
    if !credentials_of(raw).signed_in() {
        return None;
    }
    let oauth = raw.get(OAUTH_KEY)?;
    let part = |key: &str| {
        oauth
            .get(key)
            .map(|value| value.to_string())
            .unwrap_or_default()
    };
    let text = format!(
        "{}\0{}\0{}",
        part("accessToken"),
        part("refreshToken"),
        part("expiresAt")
    );
    let digest = Sha256::digest(text.as_bytes());
    Some(digest.iter().take(8).map(|b| format!("{b:02x}")).collect())
}

/// The first 16 hex of the SHA-256 of `raw`'s refresh token, when it has
/// one: what tells two chains apart without keeping a token (G7).
pub fn chain_id(raw: Option<&Raw>) -> Option<String> {
    let token = raw?
        .get(OAUTH_KEY)?
        .get("refreshToken")?
        .as_str()
        .filter(|token| !token.trim().is_empty())?;
    let digest = Sha256::digest(token.as_bytes());
    Some(digest.iter().take(8).map(|b| format!("{b:02x}")).collect())
}

/// The Keychain service Claude Code keeps an account's sign-in under:
/// `config_dir_env` is the `CLAUDE_CONFIG_DIR` it runs with (`None` for the
/// default account). Claude Code 2.1.322 (`I6()` in its Keychain chunk)
/// hashes `Be()`, which it imports from its config chunk as
/// `(CLAUDE_CONFIG_DIR ?? ~/.claude).normalize("NFC")`: the exact string
/// set, NFC-normalised, never resolved to a real path. Pass the very string
/// put in `CLAUDE_CONFIG_DIR`.
pub fn keychain_service(config_dir_env: Option<&str>) -> String {
    match config_dir_env.filter(|dir| !dir.is_empty()) {
        None => "Claude Code-credentials".to_string(),
        Some(dir) => {
            let dir = icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(dir);
            let digest = Sha256::digest(dir.as_bytes());
            let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
            format!("Claude Code-credentials-{hex}")
        }
    }
}

/// The user name Claude Code gives the Keychain: `$USER` when set and not
/// empty, else this process's user's login name (`process.env.USER ||
/// os.userInfo().username` in Claude Code 2.1.322): a worker started
/// without `USER` (launchd) still finds the sign-in.
pub fn claude_user() -> Option<String> {
    std::env::var("USER")
        .ok()
        .filter(|user| !user.is_empty())
        .or_else(login_name)
}

/// The login name of this process's (effective) user, from the user
/// database.
fn login_name() -> Option<String> {
    let mut buf: Vec<libc::c_char> = vec![0; 4096];
    // SAFETY: an all-zero passwd is a valid value for getpwuid_r to fill.
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: getpwuid_r fills `pwd`, with its strings in `buf` (of the
    // length given), and points `found` at `pwd`, or leaves it null.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    if rc != 0 || found.is_null() || pwd.pw_name.is_null() {
        return None;
    }
    // SAFETY: `pw_name` points at a NUL-terminated string inside `buf`,
    // which lives until the end of this function.
    let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) };
    name.to_str().ok().map(str::to_string)
}

/// The Keychain account Claude Code uses for `user` ([`claude_user`]), or
/// `claude-code-user` when there is none or it has other characters than
/// `[A-Za-z0-9._-]`.
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

/// Where a sign-in is read from and written to; a trait so tests never
/// touch a Keychain.
pub trait SecretStore: Send + Sync {
    /// The whole store of the account in `dir`, `env_dir` being the
    /// `CLAUDE_CONFIG_DIR` it runs with (`None` for the global slot,
    /// `~/.claude`). `Ok(None)` when it has none. Blocking.
    fn read_raw(&self, dir: &Path, env_dir: Option<&str>) -> Result<Option<Raw>, String>;

    /// Replace the whole store with `value` (G3 steps 3–5). Blocking. Never
    /// logs or returns what it holds.
    fn write_raw(&self, dir: &Path, env_dir: Option<&str>, value: &Raw) -> Result<(), String>;

    /// The sign-in of the account in `dir`. `Ok(None)` when it has none.
    fn read(&self, dir: &Path, env_dir: Option<&str>) -> Result<Option<Credentials>, String> {
        Ok(self.read_raw(dir, env_dir)?.as_ref().map(credentials_of))
    }

    /// How long a running Claude Code may take to see a change of its store,
    /// in seconds: 0 for a file (it looks on every request), 30 for the
    /// Keychain (Claude Code keeps what it read for 30 seconds).
    fn delay_s(&self) -> u32 {
        0
    }
}

/// `<dir>/.credentials.json`.
pub struct FileStore;

impl SecretStore for FileStore {
    fn read_raw(&self, dir: &Path, _env_dir: Option<&str>) -> Result<Option<Raw>, String> {
        read_file_raw(&dir.join(CREDENTIALS_FILE))
    }

    fn write_raw(&self, dir: &Path, _env_dir: Option<&str>, value: &Raw) -> Result<(), String> {
        write_file_raw(&dir.join(CREDENTIALS_FILE), value)
    }
}

/// The sign-in file's name in a config folder.
pub const CREDENTIALS_FILE: &str = ".credentials.json";

fn read_file_raw(path: &Path) -> Result<Option<Raw>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| match value {
                serde_json::Value::Object(map) => Some(Some(map)),
                _ => None,
            })
            .ok_or_else(|| "The sign-in can't be read.".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("The sign-in can't be read.".to_string()),
    }
}

/// Write a sign-in file as Claude Code does: a temporary file next to it
/// (`O_EXCL`, `0600`), written, synced, renamed over it, then `chmod 0600`.
/// A new file (new inode, new modification time): what running sessions
/// look for on their next request.
pub fn write_file_raw(path: &Path, value: &Raw) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let folder = path
        .parent()
        .ok_or_else(|| "A sign-in file needs a folder.".to_string())?;
    super::registry::make_private_dir(folder)?;
    let text =
        serde_json::to_vec(value).map_err(|_| "The sign-in can't be written.".to_string())?;
    let temp = folder.join(format!(
        "{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(CREDENTIALS_FILE),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&text)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!(
            "Could not save {} ({}).",
            path.display(),
            error.kind()
        ));
    }
    Ok(())
}

/// macOS: the login Keychain through `security`, then the file (Claude
/// Code's own order: the Keychain item, else `<dir>/.credentials.json`).
pub struct KeychainStore {
    /// `/usr/bin/security`; tests give a fake one, so no test ever asks
    /// the real Keychain.
    pub program: PathBuf,
    pub account: String,
}

/// How long `security` may take (it waits while the Keychain is locked).
const SECURITY_TIMEOUT: Duration = Duration::from_secs(5);
/// `security`'s exit code for an item that is not there.
const ITEM_NOT_FOUND: i32 = 44;
/// The longest line `security -i` takes on its standard input (Claude Code
/// 2.1.322 passes longer ones as arguments instead).
const INTERACTIVE_MAX: usize = 4032;

/// Run `security` with `args`, `input` on its standard input, within
/// [`SECURITY_TIMEOUT`]. Returns its exit code and what it printed.
fn run_security(
    program: &Path,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<(Option<i32>, String), String> {
    use std::io::{Read, Write};
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "The Keychain can't be reached.".to_string())?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        // A failed write shows in the exit code.
        let _ = stdin.write_all(input);
    }
    // Read while it runs: a store larger than the pipe's buffer (16 KiB on
    // macOS) would block `security` on its write until it is read.
    let reader = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut out = String::new();
            let _ = pipe.read_to_string(&mut out);
            out
        })
    });
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > SECURITY_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                // Its end of the pipe is closed: the reader ends.
                drop(reader);
                return Err("The Keychain did not answer.".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("The Keychain can't be reached.".to_string());
            }
        }
    };
    let stdout = reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    Ok((status.code(), stdout))
}

/// What `security -i` reads to store `secret` (hex) as the item of
/// `account` and `service`, as Claude Code writes it.
pub fn keychain_add_line(account: &str, service: &str, secret_hex: &str) -> String {
    format!("add-generic-password -U -a \"{account}\" -s \"{service}\" -X \"{secret_hex}\"\n")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl SecretStore for KeychainStore {
    fn read_raw(&self, dir: &Path, env_dir: Option<&str>) -> Result<Option<Raw>, String> {
        let service = keychain_service(env_dir);
        let (code, stdout) = run_security(
            &self.program,
            &[
                "find-generic-password",
                "-a",
                &self.account,
                "-w",
                "-s",
                &service,
            ],
            None,
        )?;
        match code {
            Some(0) => match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                Ok(serde_json::Value::Object(map)) => Ok(Some(map)),
                _ => Err("The sign-in can't be read.".to_string()),
            },
            // Claude Code falls back to the file when the Keychain has none.
            Some(ITEM_NOT_FOUND) => read_file_raw(&dir.join(CREDENTIALS_FILE)),
            _ => Err("The Keychain refused to give the sign-in.".to_string()),
        }
    }

    fn write_raw(&self, dir: &Path, env_dir: Option<&str>, value: &Raw) -> Result<(), String> {
        let service = keychain_service(env_dir);
        let json =
            serde_json::to_vec(value).map_err(|_| "The sign-in can't be written.".to_string())?;
        let secret = hex(&json);
        let line = keychain_add_line(&self.account, &service, &secret);
        // Claude Code's own way: the secret on standard input, never on the
        // command line, unless it is longer than `security -i` takes (4032
        // bytes, Claude Code's `ve`); then the whole store, hex-encoded with
        // its refresh tokens, goes as arguments, where another process of
        // the same user could read it while `security` runs. That is the
        // same as Claude Code does, and only for a store over 4 KB.
        let (code, _) = if line.len() <= INTERACTIVE_MAX {
            run_security(&self.program, &["-i"], Some(line.as_bytes()))?
        } else {
            run_security(
                &self.program,
                &[
                    "add-generic-password",
                    "-U",
                    "-a",
                    &self.account,
                    "-s",
                    &service,
                    "-X",
                    &secret,
                ],
                None,
            )?
        };
        if code != Some(0) {
            return Err("The Keychain refused to keep the sign-in.".to_string());
        }
        // A plaintext file next to the item is the store's fallback, and
        // its modification time is what sessions watch while it exists: it
        // gets the same change. None is ever made.
        let file = dir.join(CREDENTIALS_FILE);
        if std::fs::symlink_metadata(&file).is_ok() {
            write_file_raw(&file, value)?;
        }
        Ok(())
    }

    fn delay_s(&self) -> u32 {
        30
    }
}

/// The real `security`.
pub const SECURITY: &str = "/usr/bin/security";

/// This platform's store.
pub fn platform_store() -> Box<dyn SecretStore> {
    if cfg!(target_os = "macos") {
        Box::new(KeychainStore {
            program: PathBuf::from(SECURITY),
            account: keychain_account(claude_user().as_deref()),
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
        // NFC: "é" written as e + U+0301 names the same item as U+00E9.
        assert_eq!(
            keychain_service(Some("/Users/me/caf\u{65}\u{301}")),
            keychain_service(Some("/Users/me/caf\u{e9}"))
        );
        let digest = Sha256::digest("/Users/me/caf\u{e9}".as_bytes());
        assert_eq!(
            keychain_service(Some("/Users/me/caf\u{65}\u{301}")),
            format!(
                "Claude Code-credentials-{:02x}{:02x}{:02x}{:02x}",
                digest[0], digest[1], digest[2], digest[3]
            )
        );
        assert_eq!(keychain_account(Some("sergio")), "sergio");
        assert_eq!(keychain_account(Some("José")), "claude-code-user");
        assert_eq!(keychain_account(None), "claude-code-user");
    }

    #[test]
    fn the_login_name_is_this_user_s() {
        let expected = std::process::Command::new("id")
            .arg("-un")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
        assert_eq!(login_name(), expected);
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
    fn a_written_store_is_a_new_private_file_with_only_what_was_given() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir =
            std::env::temp_dir().join(format!("tmux-creds-w-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CREDENTIALS_FILE);
        std::fs::write(&path, FILE).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let before = std::fs::metadata(&path).unwrap();
        let mut raw = FileStore.read_raw(&dir, None).unwrap().unwrap();
        assert_eq!(chain_id(Some(&raw)).unwrap().len(), 16);
        raw.insert(
            OAUTH_KEY.into(),
            serde_json::json!({"accessToken": "demo-b", "refreshToken": "rb", "expiresAt": 1}),
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        FileStore.write_raw(&dir, None, &raw).unwrap();
        let after = std::fs::metadata(&path).unwrap();
        assert_ne!(before.ino(), after.ino(), "a new file");
        assert!(after.modified().unwrap() > before.modified().unwrap());
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
        let back = FileStore.read_raw(&dir, None).unwrap().unwrap();
        assert_eq!(
            back["mcpOAuth"],
            serde_json::json!({"x": {"accessToken": "other"}})
        );
        assert_eq!(back[OAUTH_KEY]["refreshToken"], "rb");
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .count();
        assert_eq!(leftovers, 0);
        // A cleared sign-in (Claude Code after `invalid_grant`) is no sign-in.
        let cleared = parse_credentials(
            r#"{"claudeAiOauth":{"accessToken":"","refreshToken":"","expiresAt":0}}"#,
        )
        .unwrap();
        assert!(!cleared.signed_in());
        assert!(chain_id(None).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn security_printing_more_than_a_pipe_holds_is_read_whole() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("tmux-creds-big-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        // A store with many MCP sign-ins, printed whole: 200 KB, more than
        // any pipe buffer.
        let program = dir.join("security");
        std::fs::write(
            &program,
            "#!/bin/sh\nhead -c 200000 /dev/zero | tr '\\0' 'a'\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        let (code, out) = run_security(&program, &["find-generic-password"], None).unwrap();
        assert_eq!(code, Some(0));
        assert_eq!(out.len(), 200_000);
        assert!(started.elapsed() < SECURITY_TIMEOUT);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The fake `security` of `tests/support`, in a folder of its own.
    pub(crate) fn fake_security(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let program = dir.join("security");
        std::fs::write(&program, include_str!("../../tests/support/fake-security")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    #[test]
    fn the_keychain_store_goes_through_security_with_the_secret_on_stdin() {
        let base =
            std::env::temp_dir().join(format!("tmux-creds-k-{}", uuid::Uuid::new_v4().simple()));
        let program = fake_security(&base.join("keychain"));
        assert_ne!(program, Path::new(SECURITY));
        let store = KeychainStore {
            program: program.clone(),
            account: "tester".into(),
        };
        let slot = base.join("home/.claude");
        let folder = base.join("accounts/personal");
        std::fs::create_dir_all(&slot).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        let env = folder.to_string_lossy().into_owned();
        // Nothing in the Keychain, no file: no sign-in.
        assert_eq!(store.read_raw(&slot, None).unwrap(), None);
        // The file is the fallback.
        std::fs::write(folder.join(CREDENTIALS_FILE), FILE).unwrap();
        assert!(store
            .read(&folder, Some(&env))
            .unwrap()
            .unwrap()
            .signed_in());
        let raw: Raw = serde_json::from_str(FILE).unwrap();
        store.write_raw(&slot, None, &raw).unwrap();
        assert!(
            !slot.join(CREDENTIALS_FILE).exists(),
            "no plaintext file is ever made"
        );
        assert_eq!(store.read_raw(&slot, None).unwrap().unwrap(), raw);
        // A folder with a plaintext file gets the change there too.
        let mut other = raw.clone();
        other.remove(OAUTH_KEY);
        store.write_raw(&folder, Some(&env), &other).unwrap();
        assert_eq!(store.read_raw(&folder, Some(&env)).unwrap().unwrap(), other);
        assert_eq!(
            FileStore.read_raw(&folder, None).unwrap().unwrap(),
            other,
            "the fallback file follows"
        );
        let log = std::fs::read_to_string(base.join("keychain/calls.log")).unwrap();
        assert!(
            log.contains("add-stdin tester Claude Code-credentials\n"),
            "{log}"
        );
        assert!(log.contains(&format!(
            "add-stdin tester {}",
            keychain_service(Some(&env))
        )));
        assert!(
            !log.contains("add-argv"),
            "the secret never went on argv: {log}"
        );
        assert!(!log.contains("demo-work"));
        assert_eq!(store.delay_s(), 30);
        assert_eq!(FileStore.delay_s(), 0);
        // A Keychain that refuses: an error, nothing written.
        let failing = std::process::Command::new(&program)
            .args([
                "add-generic-password",
                "-U",
                "-a",
                "x",
                "-s",
                "y",
                "-X",
                "00",
            ])
            .env("FAKE_SECURITY_FAIL", "1")
            .status()
            .unwrap();
        assert_eq!(failing.code(), Some(36));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_add_line_is_claude_code_s() {
        assert_eq!(
            keychain_add_line("me", "Claude Code-credentials", "7b7d"),
            "add-generic-password -U -a \"me\" -s \"Claude Code-credentials\" -X \"7b7d\"\n"
        );
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
