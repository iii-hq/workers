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
//!
//! A fresh `data_dir` — every deploy has one — reads as `new`, so `get` also
//! reports whether the wizard may open by itself here at all: not where the
//! ADE configuration sets `onboarding.auto_open: false`, nor where the
//! worker's environment sets `III_CONSOLE_ONBOARDING_AUTO_OPEN=false`.
//!
//! `console::onboarding::prompts` lists the example prompts the wizard's
//! last step offers, from `onboarding.yaml` at the project root (the Compose
//! directory, `III_COMPOSE_DIR`), which the project's template ships. It is
//! read on every call, so an edit shows the next time the step opens. Every
//! entry is validated on its own: an invalid one is skipped with a warning,
//! and a missing or broken file lists none — it never fails the wizard.

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

/// Turns the wizard's auto-open off for one environment (a deploy) when it
/// reads `false`, `0`, `off` or `no`, whatever the configuration says.
pub const AUTO_OPEN_ENV: &str = "III_CONSOLE_ONBOARDING_AUTO_OPEN";

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

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetOutput {
    #[serde(flatten)]
    pub state: OnboardingState,
    /// Whether the wizard may open by itself on this ADE: `false` when the
    /// configuration's `onboarding.auto_open` or `III_CONSOLE_ONBOARDING_AUTO_OPEN`
    /// says so, or when the configuration could not be read. Opening it from
    /// the command palette is never affected.
    pub auto_open: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetInput {
    pub status: OnboardingStatus,
    /// Replaces the stored summary when present.
    #[serde(default)]
    pub summary: Option<Value>,
}

/// File at the project root declaring the setup wizard's example prompts.
pub const PROMPTS_FILE: &str = "onboarding.yaml";

/// A file larger than this is not read: it is not a list of a few prompts.
const MAX_PROMPTS_FILE_BYTES: u64 = 256 * 1024;
const MAX_PROMPTS: usize = 12;
const MAX_PROMPT_MODELS: usize = 16;
const MAX_TITLE_CHARS: usize = 80;
const MAX_DESCRIPTION_CHARS: usize = 280;
const MAX_PROMPT_CHARS: usize = 4_000;
const MAX_ID_CHARS: usize = 128;
/// The agent profile a prompt runs with when it names none.
const DEFAULT_PROMPT_AGENT: &str = "default";
/// The ADE thinking levels a prompt may ask for; none keeps the default.
const PROMPT_EFFORTS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "off"];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptsInput {}

/// One entry of a prompt's model priority list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PromptModel {
    /// Router provider id (`claude-code`, `anthropic`).
    pub provider: String,
    /// The model id, bare or as the provider lists it (`claude-code/claude-sonnet-5-5`).
    pub model: String,
    /// An ADE thinking level; absent keeps the chat's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// An example prompt, as the wizard shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ExamplePrompt {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Agent profile id the new chat selects.
    pub agent: String,
    /// The text prefilled in the composer; the user sends it.
    pub prompt: String,
    /// Model priority list: the first one this machine has wins.
    pub models: Vec<PromptModel>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptsOutput {
    pub prompts: Vec<ExamplePrompt>,
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

    iii.register_function(
        "console::onboarding::prompts",
        RegisterFunction::new_async(|_: PromptsInput| async move {
            let path = iii_worker_paths::project_path(PROMPTS_FILE);
            Ok::<_, Error>(PromptsOutput {
                prompts: load_prompts(&path).await,
            })
        })
        .description(
            "List the example prompts the project's onboarding.yaml declares for the last step \
             of the ADE setup wizard: each one's text, agent profile and model priority list. \
             Empty when the file is missing or invalid.",
        )
        .metadata(json!({ "internal": true })),
    );

    // Serializes get-modify-set so two tabs finishing the wizard together
    // cannot interleave a stale copy.
    let lock = Arc::new(Mutex::new(()));
    let store = workspace.clone();
    let client = iii.clone();
    iii.register_function(
        "console::onboarding::get",
        RegisterFunction::new_async(move |_: GetInput| {
            let store = store.clone();
            let client = client.clone();
            async move {
                let dir = store.dir().await;
                let state = load_state(&dir).await;
                let environment = std::env::var(AUTO_OPEN_ENV).ok();
                // A configuration that cannot be read says nothing about the
                // switch: stay closed rather than open over a deployed ADE.
                let auto_open = match crate::configuration::existing_value(&client).await {
                    Ok(value) => auto_open(value.as_ref(), environment.as_deref()),
                    Err(error) => {
                        tracing::warn!(%error, "cannot read the ADE configuration; the setup wizard will not open by itself");
                        false
                    }
                };
                Ok::<_, Error>(GetOutput { state, auto_open })
            }
        })
        .description(
            "Read whether the ADE setup wizard was finished or dismissed on this machine, and \
             whether it may open by itself here.",
        )
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

/// The wizard may open by itself unless the environment variable or the
/// configuration's `onboarding.auto_open` turns it off.
fn auto_open(configuration: Option<&Value>, environment: Option<&str>) -> bool {
    let off_by_environment = environment.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "off" | "no"
        )
    });
    let off_by_configuration = configuration
        .and_then(|value| value.pointer("/onboarding/auto_open"))
        .and_then(Value::as_bool)
        == Some(false);
    !off_by_environment && !off_by_configuration
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

/// The example prompts in `path`; none when it is missing, too large or not
/// a prompts file.
async fn load_prompts(path: &Path) -> Vec<ExamplePrompt> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) if metadata.len() > MAX_PROMPTS_FILE_BYTES => {
            tracing::warn!(
                path = %path.display(),
                bytes = metadata.len(),
                "the example prompts file is too large; the setup wizard shows none"
            );
            return Vec::new();
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cannot read the example prompts file");
            return Vec::new();
        }
    }
    match tokio::fs::read_to_string(path).await {
        Ok(text) => parse_prompts(&text),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cannot read the example prompts file");
            Vec::new()
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawPromptsFile {
    #[serde(default)]
    prompts: Option<Vec<serde_yaml::Value>>,
}

#[derive(Debug, Deserialize)]
struct RawPrompt {
    title: Option<String>,
    description: Option<String>,
    agent: Option<String>,
    prompt: Option<String>,
    #[serde(default)]
    models: Option<Vec<serde_yaml::Value>>,
}

#[derive(Debug, Deserialize)]
struct RawPromptModel {
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
}

/// Every valid prompt in an `onboarding.yaml` body, in file order, at most
/// [`MAX_PROMPTS`]. An invalid entry is skipped with a warning; a body that
/// is not a prompts file lists none.
fn parse_prompts(text: &str) -> Vec<ExamplePrompt> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let file: RawPromptsFile = match serde_yaml::from_str(text) {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(%error, "{PROMPTS_FILE} is not a valid prompts file; the setup wizard shows no example prompts");
            return Vec::new();
        }
    };
    let entries = file.prompts.unwrap_or_default();
    let total = entries.len();
    let mut prompts = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        if prompts.len() == MAX_PROMPTS {
            tracing::warn!(
                skipped = total - index,
                "{PROMPTS_FILE} lists more than {MAX_PROMPTS} prompts; the rest are skipped"
            );
            break;
        }
        match prompt_entry(entry) {
            Ok(prompt) => prompts.push(prompt),
            Err(reason) => {
                tracing::warn!(index, %reason, "skipping an example prompt in {PROMPTS_FILE}")
            }
        }
    }
    prompts
}

fn prompt_entry(value: serde_yaml::Value) -> Result<ExamplePrompt, String> {
    let raw: RawPrompt = serde_yaml::from_value(value).map_err(|error| error.to_string())?;
    let title = one_line(raw.title.as_deref()).ok_or("it has no title")?;
    within("title", &title, MAX_TITLE_CHARS)?;
    let description = one_line(raw.description.as_deref());
    if let Some(description) = &description {
        within("description", description, MAX_DESCRIPTION_CHARS)?;
    }
    let prompt = raw
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .ok_or_else(|| format!("{title:?} has no prompt"))?
        .to_string();
    within("prompt", &prompt, MAX_PROMPT_CHARS)?;
    let agent = match one_line(raw.agent.as_deref()) {
        None => DEFAULT_PROMPT_AGENT.to_string(),
        Some(agent) if is_profile_id(&agent) => agent,
        Some(agent) => {
            return Err(format!(
                "{title:?} names an invalid agent profile {agent:?}"
            ))
        }
    };
    let raw_models = raw.models.unwrap_or_default();
    if raw_models.len() > MAX_PROMPT_MODELS {
        tracing::warn!(
            prompt = %title,
            "an example prompt lists more than {MAX_PROMPT_MODELS} models; the rest are skipped"
        );
    }
    let models = raw_models
        .into_iter()
        .take(MAX_PROMPT_MODELS)
        .enumerate()
        .filter_map(|(index, value)| match prompt_model(value) {
            Ok(model) => Some(model),
            Err(reason) => {
                tracing::warn!(prompt = %title, index, %reason, "skipping a model of an example prompt");
                None
            }
        })
        .collect();
    Ok(ExamplePrompt {
        title,
        description,
        agent,
        prompt,
        models,
    })
}

fn prompt_model(value: serde_yaml::Value) -> Result<PromptModel, String> {
    let raw: RawPromptModel = serde_yaml::from_value(value).map_err(|error| error.to_string())?;
    let provider = model_id("provider", raw.provider.as_deref())?;
    let model = model_id("model", raw.model.as_deref())?;
    let effort = match raw.effort.as_deref().map(str::trim) {
        None | Some("") | Some("default") => None,
        Some(effort) => {
            let effort = effort.to_ascii_lowercase();
            if !PROMPT_EFFORTS.contains(&effort.as_str()) {
                return Err(format!(
                    "effort {effort:?} is not one of {}",
                    PROMPT_EFFORTS.join(", ")
                ));
            }
            Some(effort)
        }
    };
    Ok(PromptModel {
        provider,
        model,
        effort,
    })
}

/// A provider or model id: present, one token, no longer than ids get.
fn model_id(field: &str, value: Option<&str>) -> Result<String, String> {
    let value = value.map(str::trim).unwrap_or_default();
    if value.is_empty() {
        return Err(format!("it has no {field}"));
    }
    if value.chars().count() > MAX_ID_CHARS
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("{field} {value:?} is not an id"));
    }
    Ok(value.to_string())
}

/// An agent profile id as the Directory names them (its file name).
fn is_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_ID_CHARS
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Trimmed, inner whitespace collapsed; `None` when blank.
fn one_line(value: Option<&str>) -> Option<String> {
    let line = value?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!line.is_empty()).then_some(line)
}

fn within(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.chars().count() > max {
        return Err(format!("its {field} is longer than {max} characters"));
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

    #[test]
    fn auto_open_is_on_unless_configuration_or_environment_turns_it_off() {
        assert!(auto_open(None, None));
        assert!(auto_open(Some(&json!({ "http_port": 3113 })), None));
        assert!(auto_open(
            Some(&json!({ "onboarding": { "auto_open": true } })),
            Some("true")
        ));
        assert!(!auto_open(
            Some(&json!({ "onboarding": { "auto_open": false } })),
            None
        ));
        // A deploy turns it off for its own environment, whatever is committed.
        for off in ["false", "0", "off", "NO", " False "] {
            assert!(!auto_open(None, Some(off)), "{off:?} should turn it off");
        }
        // Only `false` turns it off: a misspelling is not a reason to hide setup.
        assert!(auto_open(
            Some(&json!({ "onboarding": { "auto_open": "false" } })),
            Some("")
        ));
    }

    #[test]
    fn get_reports_the_state_beside_the_switch() {
        let output = GetOutput {
            state: OnboardingState::default(),
            auto_open: false,
        };
        let value = serde_json::to_value(&output).unwrap();
        assert_eq!(value["status"], "new");
        assert_eq!(value["updated_at"], 0);
        assert_eq!(value["auto_open"], false);
    }

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

    const PROMPTS: &str = r#"
# Example prompts for the setup wizard.
prompts:
  - title: Build a TODO app
    description: >-
      A todo list with notes
      and a public page
    agent: ade-worker-builder
    prompt: >-
      Build a TODO app.
      Show how many todos are still open.
    models:
      - { provider: claude-code, model: claude-sonnet-5-5, effort: medium }
      - { provider: openai-codex, model: codex/gpt-6.1-sol, effort: XHigh }
      - { provider: openai, model: gpt-6.1-sol }
  - title: Explain this project
    prompt: |
      Explain this project.
      Then help me decide what to work on next.
"#;

    #[test]
    fn parses_prompts_with_their_profile_and_model_priority() {
        let prompts = parse_prompts(PROMPTS);
        assert_eq!(prompts.len(), 2);
        assert_eq!(
            prompts[0],
            ExamplePrompt {
                title: "Build a TODO app".into(),
                description: Some("A todo list with notes and a public page".into()),
                agent: "ade-worker-builder".into(),
                prompt: "Build a TODO app. Show how many todos are still open.".into(),
                models: vec![
                    PromptModel {
                        provider: "claude-code".into(),
                        model: "claude-sonnet-5-5".into(),
                        effort: Some("medium".into()),
                    },
                    PromptModel {
                        provider: "openai-codex".into(),
                        model: "codex/gpt-6.1-sol".into(),
                        effort: Some("xhigh".into()),
                    },
                    PromptModel {
                        provider: "openai".into(),
                        model: "gpt-6.1-sol".into(),
                        effort: None,
                    },
                ],
            }
        );
        // No agent runs the default profile; a literal block keeps its lines.
        assert_eq!(prompts[1].agent, "default");
        assert_eq!(prompts[1].description, None);
        assert_eq!(
            prompts[1].prompt,
            "Explain this project.\nThen help me decide what to work on next."
        );
        assert!(prompts[1].models.is_empty());
        let wire = serde_json::to_value(&prompts[1]).unwrap();
        assert!(wire.get("description").is_none());
    }

    #[test]
    fn skips_invalid_entries_and_keeps_the_rest() {
        let long_title = "t".repeat(MAX_TITLE_CHARS + 1);
        let text = format!(
            r#"
prompts:
  - title: No prompt
  - prompt: No title
  - title: {long_title}
    prompt: Too long a title
  - title: Bad agent
    agent: "../etc/passwd"
    prompt: x
  - title: Models as a map
    prompt: x
    models: {{ provider: openai }}
  - "just a string"
  - title: Kept
    prompt: Keep me
    models:
      - {{ provider: openai, model: gpt-6.1-sol, effort: extreme }}
      - {{ provider: openai }}
      - {{ provider: "open ai", model: gpt }}
      - {{ provider: anthropic, model: claude-sonnet-5-5, effort: default }}
      - 42
"#
        );
        let prompts = parse_prompts(&text);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].title, "Kept");
        // Only the model entry that validates is kept; `default` means none.
        assert_eq!(
            prompts[0].models,
            vec![PromptModel {
                provider: "anthropic".into(),
                model: "claude-sonnet-5-5".into(),
                effort: None,
            }]
        );
    }

    #[test]
    fn caps_how_many_prompts_and_models_it_lists() {
        let models = (0..MAX_PROMPT_MODELS + 4)
            .map(|index| format!("      - {{ provider: p, model: m{index} }}\n"))
            .collect::<String>();
        let mut text = String::from("prompts:\n");
        for index in 0..MAX_PROMPTS + 3 {
            text.push_str(&format!(
                "  - title: Prompt {index}\n    prompt: Do {index}\n    models:\n{models}"
            ));
        }
        let prompts = parse_prompts(&text);
        assert_eq!(prompts.len(), MAX_PROMPTS);
        assert_eq!(prompts[0].models.len(), MAX_PROMPT_MODELS);
        assert_eq!(
            prompts[MAX_PROMPTS - 1].title,
            format!("Prompt {}", MAX_PROMPTS - 1)
        );
    }

    #[test]
    fn a_broken_or_foreign_file_lists_no_prompts() {
        for text in [
            "",
            "   \n",
            "prompts: [",
            "- title: a list at the top",
            "prompts: not a list",
            "other: 1",
            "prompts:",
        ] {
            assert!(parse_prompts(text).is_empty(), "{text:?} should list none");
        }
    }

    #[tokio::test]
    async fn loads_prompts_from_the_file_and_none_without_it() {
        let dir = scratch_dir("prompts");
        let path = dir.join(PROMPTS_FILE);
        assert!(load_prompts(&path).await.is_empty());
        std::fs::write(&path, PROMPTS).unwrap();
        assert_eq!(load_prompts(&path).await.len(), 2);
        let too_large = "#".repeat(MAX_PROMPTS_FILE_BYTES as usize + 1);
        std::fs::write(&path, too_large).unwrap();
        assert!(load_prompts(&path).await.is_empty());
        // A directory where the file should be is no file at all.
        let as_dir = dir.join("as-dir");
        std::fs::create_dir_all(as_dir.join(PROMPTS_FILE)).unwrap();
        assert!(load_prompts(&as_dir.join(PROMPTS_FILE)).await.is_empty());
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
