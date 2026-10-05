//! First-run onboarding: what the setup wizard needs to know about the host,
//! and whether this machine has already been through it.
//!
//! `console::onboarding::scan` looks for the Codex and Claude Code CLIs (the
//! `PATH` plus their usual install locations) and for their sign-in, so the
//! wizard can offer the subscription providers that need no API key. It
//! reports presence and paths only — a credential's content never leaves the
//! file it lives in. The criteria match the provider workers that consume the
//! same files: `provider-openai-codex` takes `auth.json` only from a ChatGPT
//! login, `provider-claude-code` takes `.credentials.json` only with a
//! `claudeAiOauth` block (or the macOS Keychain item).
//!
//! `console::onboarding::get` / `::set` keep the wizard's own progress in
//! `<data_dir>/onboarding.json`, beside the workspace layout: whether setup
//! was finished or dismissed is per-machine state (the credentials it set up
//! are per-machine too), so it stays out of the committed configuration.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::workspace_store::WorkspaceStore;

/// File name inside `data_dir` holding the wizard's progress.
pub const ONBOARDING_FILE: &str = "onboarding.json";

/// How long `<cli> --version` may take before the scan reports no version.
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// One local coding agent the wizard knows how to connect.
#[derive(Debug, Clone, Copy)]
struct Tool {
    id: &'static str,
    name: &'static str,
    binary: &'static str,
    /// Registry worker that turns this tool's sign-in into models.
    provider_worker: &'static str,
    /// Environment variable that relocates the tool's home directory.
    home_variable: &'static str,
    home_default: &'static str,
    credentials_file: &'static str,
    /// Extra install locations, relative to `$HOME`, for a `PATH` that does
    /// not carry the user's shell additions (a service manager, a GUI launch).
    home_bins: &'static [&'static str],
}

const TOOLS: [Tool; 2] = [
    Tool {
        id: "claude-code",
        name: "Claude Code",
        binary: "claude",
        provider_worker: "provider-claude-code",
        home_variable: "CLAUDE_CONFIG_DIR",
        home_default: ".claude",
        credentials_file: ".credentials.json",
        home_bins: &[".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin"],
    },
    Tool {
        id: "codex",
        name: "Codex",
        binary: "codex",
        provider_worker: "provider-openai-codex",
        home_variable: "CODEX_HOME",
        home_default: ".codex",
        credentials_file: "auth.json",
        home_bins: &[".local/bin", ".npm-global/bin", ".bun/bin", ".cargo/bin"],
    },
];

const SYSTEM_BINS: [&str; 3] = ["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScanInput {}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ToolScan {
    /// `claude-code` or `codex`.
    pub id: String,
    pub name: String,
    /// The CLI was found on this host.
    pub installed: bool,
    pub binary_path: Option<String>,
    /// First line of `<cli> --version`, when it answered in time.
    pub version: Option<String>,
    /// A sign-in the matching provider worker can use exists on this host.
    pub signed_in: bool,
    /// Where that sign-in lives, `~`-abbreviated (`~/.codex/auth.json`,
    /// `macOS Keychain`). Set whenever the file exists, signed in or not.
    pub credentials_path: Option<String>,
    /// Why a credentials file that exists does not count as signed in.
    pub sign_in_note: Option<String>,
    /// Registry worker that serves models from this sign-in.
    pub provider_worker: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScanOutput {
    pub tools: Vec<ToolScan>,
}

/// The wizard's lifecycle on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStatus {
    /// Never finished nor dismissed: the wizard opens on first load.
    #[default]
    New,
    Dismissed,
    Completed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct OnboardingState {
    #[serde(default)]
    pub status: OnboardingStatus,
    /// Milliseconds since the epoch of the last write; 0 when never written.
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    /// What the wizard set up, for the summary it shows when reopened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetInput {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetInput {
    pub status: OnboardingStatus,
    /// Replaces the stored summary when present.
    #[serde(default)]
    pub summary: Option<Value>,
}

pub fn register(iii: &Arc<IIIClient>, workspace: Arc<WorkspaceStore>) {
    iii.register_function(
        "console::onboarding::scan",
        RegisterFunction::new_async(|_: ScanInput| async move {
            Ok::<_, Error>(ScanOutput {
                tools: scan_tools().await,
            })
        })
        .description(
            "Look for the Codex and Claude Code CLIs on the ADE host and whether each is signed \
             in, for the setup wizard. Reports presence and paths only, never credentials.",
        )
        .metadata(json!({ "internal": true })),
    );

    // Serializes get-modify-set so two tabs finishing the wizard together
    // cannot interleave a stale copy.
    let lock = Arc::new(Mutex::new(()));
    let store = workspace.clone();
    iii.register_function(
        "console::onboarding::get",
        RegisterFunction::new_async(move |_: GetInput| {
            let store = store.clone();
            async move {
                let dir = store.dir().await;
                Ok::<_, Error>(load_state(&dir).await)
            }
        })
        .description("Read whether the ADE setup wizard was finished or dismissed on this machine.")
        .metadata(json!({ "internal": true })),
    );

    iii.register_function(
        "console::onboarding::set",
        RegisterFunction::new_async(move |input: SetInput| {
            let store = workspace.clone();
            let lock = lock.clone();
            async move {
                let _guard = lock.lock().await;
                let dir = store.dir().await;
                let mut state = load_state(&dir).await;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_millis() as i64);
                state.status = input.status;
                state.updated_at = now;
                match input.status {
                    OnboardingStatus::Completed => state.completed_at = Some(now),
                    // Starting over forgets what the last run set up.
                    OnboardingStatus::New => {
                        state.completed_at = None;
                        state.summary = None;
                    }
                    OnboardingStatus::Dismissed => {}
                }
                if input.summary.is_some() {
                    state.summary = input.summary;
                }
                save_state(&dir, &state).await.map_err(Error::Handler)?;
                Ok::<_, Error>(state)
            }
        })
        .description("Record that the ADE setup wizard was finished, dismissed, or reset.")
        .metadata(json!({ "internal": true })),
    );
}

async fn load_state(dir: &Path) -> OnboardingState {
    match tokio::fs::read(dir.join(ONBOARDING_FILE)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!(%error, "onboarding state is not valid JSON; treating setup as new");
            OnboardingState::default()
        }),
        Err(_) => OnboardingState::default(),
    }
}

async fn save_state(dir: &Path, state: &OnboardingState) -> Result<(), String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| format!("cannot create data_dir {}: {error}", dir.display()))?;
    let body = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("cannot serialize onboarding state: {error}"))?;
    let path = dir.join(ONBOARDING_FILE);
    let tmp = dir.join(format!("{ONBOARDING_FILE}.{}.tmp", std::process::id()));
    tokio::fs::write(&tmp, &body)
        .await
        .map_err(|error| format!("cannot write {}: {error}", tmp.display()))?;
    if let Err(error) = tokio::fs::rename(&tmp, &path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(format!("cannot move onboarding state into place: {error}"));
    }
    Ok(())
}

async fn scan_tools() -> Vec<ToolScan> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = std::env::var_os("PATH");
    let mut scans = Vec::with_capacity(TOOLS.len());
    for tool in TOOLS {
        scans.push(scan_tool(tool, path.as_ref(), home.as_deref()).await);
    }
    scans
}

async fn scan_tool(tool: Tool, path: Option<&OsString>, home: Option<&Path>) -> ToolScan {
    let binary = find_binary(tool, path, home);
    let version = match &binary {
        Some(binary) => read_version(binary).await,
        None => None,
    };
    let tool_home = std::env::var_os(tool.home_variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(tool.home_default)));
    let credentials = tool_home.map(|dir| dir.join(tool.credentials_file));
    let sign_in = match &credentials {
        Some(file) => {
            let file = file.clone();
            let id = tool.id;
            tokio::task::spawn_blocking(move || read_sign_in(id, &file))
                .await
                .unwrap_or(SignIn::Missing)
        }
        None => SignIn::Missing,
    };
    let (signed_in, sign_in_note) = match &sign_in {
        SignIn::SignedIn => (true, None),
        SignIn::Unusable(note) => (false, Some(note.clone())),
        SignIn::Missing => (false, None),
    };
    let mut credentials_path = match sign_in {
        SignIn::Missing => None,
        _ => credentials.as_deref().map(|file| abbreviate(file, home)),
    };
    let mut signed_in = signed_in;
    if !signed_in && tool.id == "claude-code" && macos_keychain_has_claude().await {
        signed_in = true;
        credentials_path = Some("macOS Keychain".to_string());
    }
    ToolScan {
        id: tool.id.to_string(),
        name: tool.name.to_string(),
        installed: binary.is_some(),
        binary_path: binary.as_deref().map(|file| abbreviate(file, home)),
        version,
        signed_in,
        credentials_path,
        sign_in_note: if signed_in { None } else { sign_in_note },
        provider_worker: tool.provider_worker.to_string(),
    }
}

/// The first executable `tool.binary` on `PATH`, then in the tool's usual
/// install locations, then under the newest nvm Node — a `PATH` inherited
/// from a service manager rarely carries the user's shell additions.
fn find_binary(tool: Tool, path: Option<&OsString>, home: Option<&Path>) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = path
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default();
    if let Some(home) = home {
        dirs.extend(tool.home_bins.iter().map(|dir| home.join(dir)));
        dirs.extend(nvm_bins(home));
    }
    dirs.extend(SYSTEM_BINS.iter().map(PathBuf::from));
    dirs.into_iter()
        .map(|dir| dir.join(tool.binary))
        .find(|candidate| is_executable(candidate))
}

/// `~/.nvm/versions/node/<version>/bin`, newest version first.
fn nvm_bins(home: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) else {
        return Vec::new();
    };
    let mut versions: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("bin"))
        .collect();
    versions.sort();
    versions.reverse();
    versions
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

async fn read_version(binary: &Path) -> Option<String> {
    let mut command = tokio::process::Command::new(binary);
    command
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    first_line(&String::from_utf8_lossy(&output.stdout))
}

fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.chars().take(80).collect())
}

#[derive(Debug, PartialEq, Eq)]
enum SignIn {
    Missing,
    SignedIn,
    /// The file exists but the provider worker would not use it.
    Unusable(String),
}

/// Inspect a credentials file the way its provider worker would, keeping
/// nothing but the verdict.
fn read_sign_in(tool_id: &str, file: &Path) -> SignIn {
    let Ok(contents) = std::fs::read_to_string(file) else {
        return SignIn::Missing;
    };
    let Ok(root) = serde_json::from_str::<Value>(&contents) else {
        return SignIn::Unusable("the sign-in file is not valid JSON".to_string());
    };
    sign_in_verdict(tool_id, &root)
}

fn sign_in_verdict(tool_id: &str, root: &Value) -> SignIn {
    let has = |value: Option<&Value>| value.and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    match tool_id {
        "codex" => {
            if root.get("auth_mode").and_then(Value::as_str) != Some("chatgpt") {
                return SignIn::Unusable(
                    "Codex is signed in with an API key, not a ChatGPT account".to_string(),
                );
            }
            if has(root.pointer("/tokens/access_token")) {
                SignIn::SignedIn
            } else {
                SignIn::Unusable("the Codex sign-in has no session token".to_string())
            }
        }
        "claude-code" => {
            if has(root.pointer("/claudeAiOauth/accessToken")) {
                SignIn::SignedIn
            } else {
                SignIn::Unusable("the Claude Code sign-in has no subscription token".to_string())
            }
        }
        _ => SignIn::Missing,
    }
}

/// The macOS CLI keeps its login in the Keychain once it stops updating the
/// file. `security` without `-w` reports the item's existence, not its secret.
async fn macos_keychain_has_claude() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let mut command = tokio::process::Command::new("security");
    command
        .args(["find-generic-password", "-s", "Claude Code-credentials"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(VERSION_TIMEOUT, command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

/// `/home/me/.codex/auth.json` → `~/.codex/auth.json`.
fn abbreviate(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ade-onboarding-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, "#!/bin/sh\necho 'codex-cli 9.9.9'\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn codex_needs_a_chatgpt_login_with_a_token() {
        assert_eq!(
            sign_in_verdict(
                "codex",
                &json!({ "auth_mode": "chatgpt", "tokens": { "access_token": "a" } })
            ),
            SignIn::SignedIn
        );
        assert!(matches!(
            sign_in_verdict(
                "codex",
                &json!({ "auth_mode": "apikey", "OPENAI_API_KEY": "k" })
            ),
            SignIn::Unusable(_)
        ));
        assert!(matches!(
            sign_in_verdict("codex", &json!({ "auth_mode": "chatgpt", "tokens": {} })),
            SignIn::Unusable(_)
        ));
    }

    #[test]
    fn claude_code_needs_a_subscription_token() {
        assert_eq!(
            sign_in_verdict(
                "claude-code",
                &json!({ "claudeAiOauth": { "accessToken": "a", "refreshToken": "r" } })
            ),
            SignIn::SignedIn
        );
        assert!(matches!(
            sign_in_verdict("claude-code", &json!({})),
            SignIn::Unusable(_)
        ));
    }

    #[test]
    fn a_missing_or_broken_file_is_reported_without_its_content() {
        let dir = scratch_dir("files");
        assert_eq!(
            read_sign_in("codex", &dir.join("auth.json")),
            SignIn::Missing
        );
        std::fs::write(dir.join("auth.json"), "secret-token-not-json").unwrap();
        match read_sign_in("codex", &dir.join("auth.json")) {
            SignIn::Unusable(note) => assert!(!note.contains("secret-token")),
            other => panic!("expected unusable, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn finds_the_cli_on_path_before_the_fallbacks() {
        let home = scratch_dir("home");
        let on_path = scratch_dir("path");
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        executable(&home.join(".local/bin/codex"));
        executable(&on_path.join("codex"));
        let tool = TOOLS
            .iter()
            .find(|tool| tool.id == "codex")
            .copied()
            .unwrap();

        let path = std::env::join_paths([&on_path]).unwrap();
        assert_eq!(
            find_binary(tool, Some(&path), Some(&home)),
            Some(on_path.join("codex"))
        );
        // Without it on PATH, the home install location still finds it.
        let empty = OsString::new();
        assert_eq!(
            find_binary(tool, Some(&empty), Some(&home)),
            Some(home.join(".local/bin/codex"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_non_executable_file_is_not_a_cli() {
        let dir = scratch_dir("noexec");
        std::fs::write(dir.join("claude"), "").unwrap();
        assert!(!is_executable(&dir.join("claude")));
        assert!(!is_executable(&dir));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reads_the_first_version_line() {
        let dir = scratch_dir("version");
        executable(&dir.join("codex"));
        assert_eq!(
            read_version(&dir.join("codex")).await.as_deref(),
            Some("codex-cli 9.9.9")
        );
    }

    #[test]
    fn abbreviates_the_home_directory() {
        let home = Path::new("/home/me");
        assert_eq!(
            abbreviate(Path::new("/home/me/.codex/auth.json"), Some(home)),
            "~/.codex/auth.json"
        );
        assert_eq!(abbreviate(Path::new("/opt/x"), Some(home)), "/opt/x");
    }

    #[tokio::test]
    async fn state_round_trips_and_defaults_to_new() {
        let dir = scratch_dir("state");
        assert_eq!(load_state(&dir).await.status, OnboardingStatus::New);
        let state = OnboardingState {
            status: OnboardingStatus::Completed,
            updated_at: 1,
            completed_at: Some(1),
            summary: Some(json!({ "providers": ["claude-code"] })),
        };
        save_state(&dir, &state).await.unwrap();
        let loaded = load_state(&dir).await;
        assert_eq!(loaded.status, OnboardingStatus::Completed);
        assert_eq!(loaded.summary, state.summary);
        std::fs::write(dir.join(ONBOARDING_FILE), "{not json").unwrap();
        assert_eq!(load_state(&dir).await.status, OnboardingStatus::New);
    }
}
