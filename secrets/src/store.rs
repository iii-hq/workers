//! The vault in memory: every operation runs under one lock, writes go to a
//! copy that replaces the in-memory vault only after it is on disk. The env
//! store's operations take the same lock: its grants live in the vault, its
//! values in `.env`.
use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{SecondsFormat, Utc};
use tokio::sync::Mutex;

use crate::access::is_authorized;
use crate::api::{
    DetectResponse, DetectResult, DetectedSource, KeySourceKind, SecretMeta, StatusResponse,
    StoreKind,
};
use crate::config::SecretsConfig;
use crate::crypto::MasterKey;
use crate::detect::Found;
use crate::envstore::EnvSource;
use crate::error::{codes, SecretsError};
use crate::events::{ChangeAction, ChangedEvent};
use crate::keys::{self, EnvKey, KeyRequest};
use crate::names::{self, mask, normalize_consumers, validate_name};
use crate::secret::SecretString;
use crate::vault::{EnvGrant, SecretRecord, Vault, VAULT_FILE};

pub const MAX_VALUE_BYTES: usize = 64 * 1024;
const MAX_DESCRIPTION_CHARS: usize = 1024;
pub const MAX_DETECT_NAMES: usize = 64;

/// Resolved filesystem locations for one configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorePaths {
    pub data_dir: PathBuf,
    /// Configured `key_file`, resolved.
    pub key_file: Option<PathBuf>,
    /// `${XDG_CONFIG_HOME:-~/.config}/iii/secrets`.
    pub key_dir: Option<PathBuf>,
    /// `III_COMPOSE_DIR`, when the worker runs under Compose.
    pub project_dir: Option<PathBuf>,
    /// The project's `.env`: where the env store reads and writes.
    pub dotenv: Option<PathBuf>,
}

impl StorePaths {
    pub fn from_config(config: &SecretsConfig) -> Self {
        Self {
            data_dir: iii_worker_paths::resolve_path(&config.data_dir),
            key_file: config
                .key_file
                .as_deref()
                .map(iii_worker_paths::resolve_path),
            key_dir: keys::default_key_dir(),
            project_dir: std::env::var_os(iii_worker_paths::COMPOSE_DIR_ENV)
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from),
            dotenv: Some(iii_worker_paths::resolve_path(&config.env_file)),
        }
    }

    pub fn vault_path(&self) -> PathBuf {
        self.data_dir.join(VAULT_FILE)
    }

    pub fn key_path(&self, vault_id: &str) -> Option<PathBuf> {
        keys::key_file_path(self.key_file.as_deref(), self.key_dir.as_deref(), vault_id)
    }
}

pub struct Store {
    env_key: EnvKey,
    /// Whether the env store falls back to this worker's environment.
    process_env: bool,
    state: Mutex<State>,
}

/// What the env store last saw of a shared variable: a digest of its value,
/// `None` while it is unset. Kept in memory only.
type Seen = Option<[u8; 32]>;

struct State {
    paths: StorePaths,
    /// Loaded on first use; `None` again after a reconfiguration.
    vault: Option<Vault>,
    key: Option<MasterKey>,
    /// Per shared variable; filled by the first scan.
    env_seen: HashMap<String, Seen>,
}

impl State {
    fn new(paths: StorePaths) -> Self {
        Self {
            paths,
            vault: None,
            key: None,
            env_seen: HashMap::new(),
        }
    }

    /// Load the vault, creating an empty one (a fresh `vault_id`) only when
    /// the file does not exist. An unreadable file stays an error.
    fn vault(&mut self) -> Result<&Vault, SecretsError> {
        if self.vault.is_none() {
            let path = self.paths.vault_path();
            let vault = match Vault::load(&path)? {
                Some(vault) => vault,
                None => {
                    let vault = Vault::new();
                    vault.save(&path)?;
                    tracing::info!(path = %path.display(), "created empty vault");
                    vault
                }
            };
            self.vault = Some(vault);
        }
        Ok(self.vault.as_ref().expect("vault loaded above"))
    }

    /// The master key for the loaded vault. `create` lets a write generate
    /// the key file, which only ever happens for a vault that has never
    /// sealed a value. The first key used is pinned in the header.
    fn key(&mut self, env: &EnvKey, create: bool) -> Result<&MasterKey, SecretsError> {
        if self.key.is_none() {
            let vault = self.vault()?.clone();
            let file = self.paths.key_path(&vault.vault_id);
            let loaded = keys::load_master_key(KeyRequest {
                env,
                file: file.as_deref(),
                key_check: vault.key_check.as_deref(),
                create: create && vault.secrets.is_empty(),
                project_dir: self.paths.project_dir.as_deref(),
            })?;
            if loaded.created {
                if let Some(file) = &file {
                    tracing::info!(path = %file.display(), "created master key file");
                }
            }
            if vault.key_check.is_none() {
                let mut next = vault;
                next.key_check = Some(loaded.key.key_check());
                self.commit(next)?;
            }
            self.key = Some(loaded.key);
        }
        Ok(self.key.as_ref().expect("key loaded above"))
    }

    fn commit(&mut self, next: Vault) -> Result<(), SecretsError> {
        next.save(&self.paths.vault_path())?;
        self.vault = Some(next);
        Ok(())
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn digest(value: &SecretString) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(value.expose().as_bytes()).into()
}

fn validate_value(value: &SecretString) -> Result<(), SecretsError> {
    if value.is_blank() {
        return Err(SecretsError::invalid_request("value must not be empty"));
    }
    if value.expose().len() > MAX_VALUE_BYTES {
        return Err(SecretsError::invalid_request(format!(
            "value must be at most {MAX_VALUE_BYTES} bytes"
        )));
    }
    Ok(())
}

/// `None`: keep. `Some(None)`: clear. `Some(Some(text))`: replace.
fn normalize_description(
    description: Option<String>,
) -> Result<Option<Option<String>>, SecretsError> {
    let Some(description) = description else {
        return Ok(None);
    };
    let description = description.trim();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(SecretsError::invalid_request(format!(
            "description must be at most {MAX_DESCRIPTION_CHARS} characters"
        )));
    }
    Ok(Some(
        (!description.is_empty()).then(|| description.to_owned()),
    ))
}

/// Validated, de-duplicated names for `secrets::detect`.
pub fn validate_detect_names(names: Vec<String>) -> Result<Vec<String>, SecretsError> {
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        validate_name(&name)?;
        if !out.contains(&name) {
            out.push(name);
        }
    }
    if out.len() > MAX_DETECT_NAMES {
        return Err(SecretsError::invalid_request(format!(
            "at most {MAX_DETECT_NAMES} names per call"
        )));
    }
    Ok(out)
}

impl Store {
    pub fn new(paths: StorePaths, env_key: EnvKey) -> Self {
        Self {
            env_key,
            process_env: true,
            state: Mutex::new(State::new(paths)),
        }
    }

    /// Read the env store from `.env` only (tests).
    pub fn without_process_env(mut self) -> Self {
        self.process_env = false;
        self
    }

    fn env_source(&self, state: &State) -> EnvSource {
        EnvSource {
            dotenv: state.paths.dotenv.clone(),
            process_env: self.process_env,
        }
    }

    /// Load (or create) the vault now and take a first look at the shared
    /// variables; returns how many vault secrets it holds.
    pub async fn open(&self) -> Result<usize, SecretsError> {
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let count = state.vault()?.secrets.len();
        self.scan_env(state);
        Ok(count)
    }

    pub async fn vault_path(&self) -> PathBuf {
        self.state.lock().await.paths.vault_path()
    }

    /// Point the store at new locations. The cached vault and key are
    /// dropped (the key is zeroized) and re-read on next use; call
    /// [`Store::env_changes`] after it to report what a new env file changed.
    pub async fn reconfigure(&self, paths: StorePaths) -> bool {
        let mut state = self.state.lock().await;
        if state.paths == paths {
            return false;
        }
        // What the env store last saw carries over, so a new env file is
        // compared with the old one and the difference reported.
        let seen = std::mem::take(&mut state.env_seen);
        *state = State::new(paths);
        state.env_seen = seen;
        if let Err(error) = state.vault() {
            tracing::warn!(error = %error, "re-configured vault is not usable yet");
        }
        true
    }

    /// Create or rotate. `consumers`/`description` of `None` keep the current
    /// values (`[]`/none for a new secret).
    pub async fn set(
        &self,
        name: &str,
        value: &SecretString,
        consumers: Option<Vec<String>>,
        description: Option<String>,
    ) -> Result<(SecretMeta, ChangedEvent), SecretsError> {
        validate_name(name)?;
        validate_value(value)?;
        let consumers = consumers.map(normalize_consumers).transpose()?;
        let description = normalize_description(description)?;
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let (sealed, fingerprint) = {
            let key = state.key(&self.env_key, true)?;
            let plaintext = value.expose().as_bytes();
            (key.seal(name, plaintext)?, key.fingerprint(plaintext))
        };
        let at = now();
        let mut next = state.vault()?.clone();
        let action = match next.secrets.get_mut(name) {
            Some(record) => {
                if let Some(consumers) = consumers {
                    record.consumers = consumers;
                }
                if let Some(description) = description {
                    record.description = description;
                }
                record.hint = mask(value.expose());
                record.fingerprint = fingerprint.clone();
                record.set_sealed(&sealed);
                record.updated_at = at.clone();
                ChangeAction::Rotated
            }
            None => {
                let mut record = SecretRecord {
                    name: name.to_owned(),
                    description: description.flatten(),
                    consumers: consumers.unwrap_or_default(),
                    created_at: at.clone(),
                    updated_at: at.clone(),
                    last_resolved_at: None,
                    last_resolved_by: None,
                    hint: mask(value.expose()),
                    fingerprint: fingerprint.clone(),
                    nonce: String::new(),
                    ciphertext: String::new(),
                };
                record.set_sealed(&sealed);
                next.secrets.insert(name.to_owned(), record);
                ChangeAction::Created
            }
        };
        let meta = next.secrets[name].meta();
        state.commit(next)?;
        Ok((
            meta,
            ChangedEvent::new(name, action, Some(fingerprint), &at),
        ))
    }

    /// Replace the allowlist.
    pub async fn access(
        &self,
        name: &str,
        consumers: Vec<String>,
    ) -> Result<(SecretMeta, ChangedEvent), SecretsError> {
        validate_name(name)?;
        let consumers = normalize_consumers(consumers)?;
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let mut next = state.vault()?.clone();
        let record = next
            .secrets
            .get_mut(name)
            .ok_or_else(|| SecretsError::not_found(name))?;
        let at = now();
        record.consumers = consumers;
        record.updated_at = at.clone();
        let meta = record.meta();
        state.commit(next)?;
        let event = ChangedEvent::new(
            name,
            ChangeAction::AccessChanged,
            meta.fingerprint.clone(),
            &at,
        );
        Ok((meta, event))
    }

    /// `None` when there was nothing to delete.
    pub async fn delete(&self, name: &str) -> Result<Option<ChangedEvent>, SecretsError> {
        validate_name(name)?;
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        if !state.vault()?.secrets.contains_key(name) {
            return Ok(None);
        }
        let mut next = state.vault()?.clone();
        next.secrets.remove(name);
        state.commit(next)?;
        Ok(Some(ChangedEvent::new(
            name,
            ChangeAction::Deleted,
            None,
            &now(),
        )))
    }

    pub async fn get(&self, name: &str) -> Result<Option<SecretMeta>, SecretsError> {
        validate_name(name)?;
        let mut state = self.state.lock().await;
        Ok(state.vault()?.secrets.get(name).map(SecretRecord::meta))
    }

    pub async fn list(&self) -> Result<Vec<SecretMeta>, SecretsError> {
        let mut state = self.state.lock().await;
        Ok(state
            .vault()?
            .secrets
            .values()
            .map(SecretRecord::meta)
            .collect())
    }

    /// The allowlist of a stored secret, `None` when it is not stored.
    pub async fn consumers_of(&self, name: &str) -> Result<Option<Vec<String>>, SecretsError> {
        let mut state = self.state.lock().await;
        Ok(state
            .vault()?
            .secrets
            .get(name)
            .map(|record| record.consumers.clone()))
    }

    /// Decrypt for `caller` (a worker name, `None` when unidentified) if the
    /// allowlist admits it, then record the resolution. The record is
    /// re-checked under the lock, so an allowlist change that lands first wins.
    pub async fn resolve(
        &self,
        name: &str,
        caller: Option<&str>,
    ) -> Result<SecretString, SecretsError> {
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let record = state
            .vault()?
            .secrets
            .get(name)
            .ok_or_else(|| SecretsError::not_found(name))?;
        if !is_authorized(&record.consumers, caller) {
            return Err(forbidden(
                StoreKind::Vault,
                name,
                caller,
                record.consumers.is_empty(),
            ));
        }
        let sealed = record.sealed()?;
        let plaintext = state.key(&self.env_key, false)?.open(name, &sealed)?;
        let value = std::str::from_utf8(&plaintext)
            .map(SecretString::from)
            .map_err(|_| SecretsError::vault(format!("stored value for `{name}` is not UTF-8")))?;
        let mut next = state.vault()?.clone();
        if let Some(record) = next.secrets.get_mut(name) {
            record.last_resolved_at = Some(now());
            record.last_resolved_by = caller.map(str::to_owned);
        }
        // Bookkeeping only: a failed write never withholds an authorized value.
        if let Err(error) = state.commit(next) {
            tracing::warn!(error = %error, "could not record the resolution");
        }
        Ok(value)
    }

    /// Combine detected values with what is stored: masked hints, and
    /// whether each detected value equals the stored one (fingerprint).
    pub async fn compare(
        &self,
        names: &[String],
        found: &[Found],
    ) -> Result<DetectResponse, SecretsError> {
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let stored: HashMap<String, (String, String)> = state
            .vault()?
            .secrets
            .iter()
            .filter(|(name, _)| names.contains(name))
            .map(|(name, record)| {
                (
                    name.clone(),
                    (record.hint.clone(), record.fingerprint.clone()),
                )
            })
            .collect();
        let key = if found.iter().any(|item| stored.contains_key(&item.name)) {
            match state.key(&self.env_key, false) {
                Ok(key) => Some(key.clone()),
                Err(error) => {
                    tracing::warn!(
                        code = error.code,
                        "cannot compare detected values with stored ones"
                    );
                    None
                }
            }
        } else {
            None
        };
        let results = names
            .iter()
            .map(|name| {
                let stored_entry = stored.get(name);
                let sources = found
                    .iter()
                    .filter(|item| &item.name == name)
                    .map(|item| DetectedSource {
                        kind: item.kind,
                        location: item.location.clone(),
                        hint: mask(item.value.expose()),
                        matches_stored: match (stored_entry, &key) {
                            (Some((_, fingerprint)), Some(key)) => {
                                &key.fingerprint(item.value.expose().as_bytes()) == fingerprint
                            }
                            _ => false,
                        },
                    })
                    .collect();
                DetectResult {
                    name: name.clone(),
                    stored: stored_entry.is_some(),
                    stored_hint: stored_entry.map(|(hint, _)| hint.clone()),
                    sources,
                }
            })
            .collect();
        Ok(DetectResponse { results })
    }

    pub async fn status(&self) -> Result<StatusResponse, SecretsError> {
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let (vault_id, count, version) = {
            let vault = state.vault()?;
            (vault.vault_id.clone(), vault.secrets.len(), vault.version)
        };
        let (key_source, key_path) = if self.env_key.is_set() {
            (KeySourceKind::Env, None)
        } else {
            (KeySourceKind::File, state.paths.key_path(&vault_id))
        };
        let env_count = state.vault()?.env.len();
        Ok(StatusResponse {
            vault_path: state.paths.vault_path().display().to_string(),
            key_source,
            key_path: key_path.map(|path| path.display().to_string()),
            count,
            version,
            env_file: state
                .paths
                .dotenv
                .as_ref()
                .map(|path| path.display().to_string()),
            env_count,
        })
    }

    /// Share an environment variable: write `value` to `.env` when one is
    /// given, then create or update its grant. `consumers`/`description` of
    /// `None` keep the current values (`[]`/none for a new grant). A
    /// variable may be shared before it is set.
    pub async fn set_env(
        &self,
        name: &str,
        value: Option<&SecretString>,
        consumers: Option<Vec<String>>,
        description: Option<String>,
    ) -> Result<(SecretMeta, ChangedEvent), SecretsError> {
        validate_name(name)?;
        if let Some(value) = value {
            validate_value(value)?;
        }
        let consumers = consumers.map(normalize_consumers).transpose()?;
        let description = normalize_description(description)?;
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let source = self.env_source(state);
        let before = source.read_one(name).map(|found| digest(&found.value));
        let mut next = state.vault()?.clone();
        if let Some(value) = value {
            source.write(name, value)?;
        }
        let current = source.read_one(name);
        let seen = current.as_ref().map(|found| digest(&found.value));
        let at = now();
        let created = !next.env.contains_key(name);
        let grant = next.env.entry(name.to_owned()).or_insert_with(|| EnvGrant {
            name: name.to_owned(),
            description: None,
            consumers: Vec::new(),
            created_at: at.clone(),
            updated_at: at.clone(),
            last_resolved_at: None,
            last_resolved_by: None,
        });
        if let Some(consumers) = consumers {
            grant.consumers = consumers;
        }
        if let Some(description) = description {
            grant.description = description;
        }
        grant.updated_at = at.clone();
        let meta = grant.meta(current.as_ref());
        state.commit(next)?;
        // The watcher must not report this write again.
        state.env_seen.insert(name.to_owned(), seen);
        // As the watcher reports it: a variable that appears is created.
        let action = if created || (before.is_none() && seen.is_some()) {
            ChangeAction::Created
        } else if before != seen {
            ChangeAction::Rotated
        } else {
            ChangeAction::AccessChanged
        };
        Ok((meta, ChangedEvent::env(name, action, &at)))
    }

    /// Stop sharing a variable. Its value stays in `.env`. `None` when it
    /// was not shared.
    pub async fn delete_env(&self, name: &str) -> Result<Option<ChangedEvent>, SecretsError> {
        validate_name(name)?;
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        if !state.vault()?.env.contains_key(name) {
            return Ok(None);
        }
        let mut next = state.vault()?.clone();
        next.env.remove(name);
        state.commit(next)?;
        state.env_seen.remove(name);
        Ok(Some(ChangedEvent::env(name, ChangeAction::Deleted, &now())))
    }

    pub async fn get_env(&self, name: &str) -> Result<Option<SecretMeta>, SecretsError> {
        validate_name(name)?;
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let source = self.env_source(state);
        Ok(state
            .vault()?
            .env
            .get(name)
            .map(|grant| grant.meta(source.read_one(name).as_ref())))
    }

    /// Every vault secret, then every shared variable with its current hint.
    pub async fn list_all(&self) -> Result<Vec<SecretMeta>, SecretsError> {
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let source = self.env_source(state);
        let vault = state.vault()?;
        let names: Vec<String> = vault.env.keys().cloned().collect();
        let values = source.read(&names);
        Ok(vault
            .secrets
            .values()
            .map(SecretRecord::meta)
            .chain(
                vault
                    .env
                    .values()
                    .map(|grant| grant.meta(values.get(&grant.name))),
            )
            .collect())
    }

    /// The allowlist of a shared variable, `None` when it is not shared.
    pub async fn env_consumers_of(&self, name: &str) -> Result<Option<Vec<String>>, SecretsError> {
        let mut state = self.state.lock().await;
        Ok(state
            .vault()?
            .env
            .get(name)
            .map(|grant| grant.consumers.clone()))
    }

    /// The variable's current value for `caller` if its grant admits it,
    /// then record the resolution. Nothing about the value is revealed to a
    /// caller the grant does not admit.
    pub async fn resolve_env(
        &self,
        name: &str,
        caller: Option<&str>,
    ) -> Result<SecretString, SecretsError> {
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let source = self.env_source(state);
        let grant = state.vault()?.env.get(name).ok_or_else(not_shared)?;
        if !is_authorized(&grant.consumers, caller) {
            return Err(forbidden(
                StoreKind::Env,
                name,
                caller,
                grant.consumers.is_empty(),
            ));
        }
        let value = source.read_one(name).ok_or_else(|| {
            SecretsError::new(
                codes::SECRET_NOT_FOUND,
                match source.dotenv_path() {
                    Some(path) => format!(
                        "environment variable `{name}` is not set in {} or in the secrets worker's environment",
                        path.display()
                    ),
                    None => format!(
                        "environment variable `{name}` is not set in the secrets worker's environment"
                    ),
                },
            )
        })?;
        let mut next = state.vault()?.clone();
        if let Some(grant) = next.env.get_mut(name) {
            grant.last_resolved_at = Some(now());
            grant.last_resolved_by = caller.map(str::to_owned);
        }
        if let Err(error) = state.commit(next) {
            tracing::warn!(error = %error, "could not record the resolution");
        }
        Ok(value.value)
    }

    /// Changes to shared variables since the last look, called when the
    /// operating system reports that `.env` changed: one event per variable
    /// set, changed or removed by an edit this worker did not make.
    pub async fn env_changes(&self) -> Vec<ChangedEvent> {
        let mut guard = self.state.lock().await;
        self.scan_env(&mut guard)
    }

    /// The `.env` the env store reads and writes.
    pub async fn dotenv_path(&self) -> Option<PathBuf> {
        self.state.lock().await.paths.dotenv.clone()
    }

    /// Compare every shared variable with what was last seen. A variable
    /// seen for the first time only sets the baseline.
    fn scan_env(&self, state: &mut State) -> Vec<ChangedEvent> {
        let names: Vec<String> = match state.vault() {
            Ok(vault) => vault.env.keys().cloned().collect(),
            Err(_) => return Vec::new(),
        };
        if names.is_empty() {
            return Vec::new();
        }
        let values = self.env_source(state).read(&names);
        let at = now();
        let mut events = Vec::new();
        for name in names {
            let seen = values.get(&name).map(|found| digest(&found.value));
            match state.env_seen.insert(name.clone(), seen) {
                Some(previous) if previous != seen => {
                    let action = match (previous, seen) {
                        (None, Some(_)) => ChangeAction::Created,
                        (Some(_), None) => ChangeAction::Deleted,
                        _ => ChangeAction::Rotated,
                    };
                    events.push(ChangedEvent::env(&name, action, &at));
                }
                _ => {}
            }
        }
        events
    }
}

/// A grant-less `env://` reference. The name is not echoed: a caller may have
/// put a raw credential after the scheme.
fn not_shared() -> SecretsError {
    SecretsError::new(
        codes::SECRET_FORBIDDEN,
        "that environment variable is not shared with any worker; allow one with secrets::access and store \"env\"",
    )
}

fn forbidden(
    store: StoreKind,
    name: &str,
    caller: Option<&str>,
    no_consumers: bool,
) -> SecretsError {
    let reference = names::reference_for(store, name);
    let message = match caller {
        _ if no_consumers => {
            format!("`{reference}` has no consumers yet; allow a worker with secrets::access")
        }
        Some(caller) => format!("worker `{caller}` is not a consumer of `{reference}`"),
        None => format!(
            "the calling worker could not be identified; only consumers of `{reference}` may resolve it"
        ),
    };
    SecretsError::new(codes::SECRET_FORBIDDEN, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::SourceKind;

    struct Fixture {
        _root: tempfile::TempDir,
        paths: StorePaths,
    }

    fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let paths = StorePaths {
            data_dir: root.path().join("project/data/secrets"),
            key_file: None,
            key_dir: Some(root.path().join("home/.config/iii/secrets")),
            project_dir: Some(root.path().join("project")),
            dotenv: Some(root.path().join("project/.env")),
        };
        Fixture { _root: root, paths }
    }

    const VALUE: &str = "sk-ant-api03-abcdefghijklmnop-9f2c";

    #[tokio::test]
    async fn set_list_resolve_round_trip_never_exposes_the_value() {
        let fx = fixture();
        let store = Store::new(fx.paths.clone(), EnvKey::default());
        let (meta, event) = store
            .set(
                "ANTHROPIC_API_KEY",
                &VALUE.into(),
                Some(vec!["llm-router".into()]),
                Some("Anthropic".into()),
            )
            .await
            .unwrap();
        assert_eq!(meta.reference, "secret://ANTHROPIC_API_KEY");
        assert_eq!(meta.hint, "sk-ant…9f2c");
        assert_eq!(meta.consumers, vec!["llm-router"]);
        assert_eq!(event.action, ChangeAction::Created);
        assert!(meta.fingerprint.is_some());
        assert_eq!(event.fingerprint, meta.fingerprint);

        let listed = serde_json::to_string(&store.list().await.unwrap()).unwrap();
        assert!(
            !listed.contains(VALUE) && !listed.contains("abcdefghijklmnop"),
            "{listed}"
        );
        let on_disk = std::fs::read_to_string(fx.paths.vault_path()).unwrap();
        assert!(!on_disk.contains("abcdefghijklmnop"), "{on_disk}");

        let value = store
            .resolve("ANTHROPIC_API_KEY", Some("llm-router"))
            .await
            .unwrap();
        assert_eq!(value.expose(), VALUE);
        let meta = store.get("ANTHROPIC_API_KEY").await.unwrap().unwrap();
        assert_eq!(meta.last_resolved_by.as_deref(), Some("llm-router"));
        assert!(meta.last_resolved_at.is_some());
    }

    #[tokio::test]
    async fn resolve_enforces_the_allowlist() {
        let fx = fixture();
        let store = Store::new(fx.paths.clone(), EnvKey::default());
        store.set("K", &VALUE.into(), None, None).await.unwrap();
        // Empty consumers: nobody.
        for caller in [Some("llm-router"), None] {
            let error = store.resolve("K", caller).await.unwrap_err();
            assert_eq!(error.code, codes::SECRET_FORBIDDEN);
            assert!(!error.message.contains("abcdefghijklmnop"));
        }
        store.access("K", vec!["llm-router".into()]).await.unwrap();
        assert!(store.resolve("K", Some("llm-router")).await.is_ok());
        assert_eq!(
            store.resolve("K", Some("harness")).await.unwrap_err().code,
            codes::SECRET_FORBIDDEN
        );
        assert_eq!(
            store
                .resolve("MISSING", Some("llm-router"))
                .await
                .unwrap_err()
                .code,
            codes::SECRET_NOT_FOUND
        );
        // Omitted consumers on rotation keep the list.
        let (meta, event) = store
            .set("K", &"sk-rotated-value-0000".into(), None, None)
            .await
            .unwrap();
        assert_eq!(meta.consumers, vec!["llm-router"]);
        assert_eq!(event.action, ChangeAction::Rotated);
        assert_eq!(
            store
                .resolve("K", Some("llm-router"))
                .await
                .unwrap()
                .expose(),
            "sk-rotated-value-0000"
        );
    }

    #[tokio::test]
    async fn rotation_changes_the_fingerprint_and_description_rules_hold() {
        let fx = fixture();
        let store = Store::new(fx.paths.clone(), EnvKey::default());
        let (first, _) = store
            .set("K", &"value-one-123456".into(), None, Some("first".into()))
            .await
            .unwrap();
        let (same, _) = store
            .set("K", &"value-one-123456".into(), None, None)
            .await
            .unwrap();
        assert_eq!(first.fingerprint, same.fingerprint);
        assert_eq!(same.description.as_deref(), Some("first"));
        let (rotated, _) = store
            .set("K", &"value-two-123456".into(), None, Some(" ".into()))
            .await
            .unwrap();
        assert_ne!(first.fingerprint, rotated.fingerprint);
        assert_eq!(rotated.description, None);
        assert_eq!(rotated.created_at, first.created_at);
    }

    #[tokio::test]
    async fn vault_persists_across_instances_with_private_permissions() {
        let fx = fixture();
        {
            let store = Store::new(fx.paths.clone(), EnvKey::default());
            store
                .set("A", &VALUE.into(), Some(vec!["llm-router".into()]), None)
                .await
                .unwrap();
            store
                .set("B", &"another-secret-xyz".into(), None, None)
                .await
                .unwrap();
            assert!(store.delete("B").await.unwrap().is_some());
            assert!(store.delete("B").await.unwrap().is_none());
        }
        let reopened = Store::new(fx.paths.clone(), EnvKey::default());
        assert_eq!(reopened.open().await.unwrap(), 1);
        assert_eq!(
            reopened
                .resolve("A", Some("llm-router"))
                .await
                .unwrap()
                .expose(),
            VALUE
        );
        let status = reopened.status().await.unwrap();
        assert_eq!(status.count, 1);
        assert_eq!(status.version, crate::vault::VAULT_VERSION);
        assert_eq!(status.key_source, KeySourceKind::File);
        let key_path = PathBuf::from(status.key_path.unwrap());
        assert!(key_path.starts_with(fx.paths.key_dir.as_ref().unwrap()));
        assert!(key_path.exists());
        #[cfg(unix)]
        {
            use crate::fsutil::{mode_of, DIR_MODE, FILE_MODE};
            assert_eq!(mode_of(&fx.paths.vault_path()).unwrap(), FILE_MODE);
            assert_eq!(mode_of(&fx.paths.data_dir).unwrap(), DIR_MODE);
            assert_eq!(mode_of(&key_path).unwrap(), FILE_MODE);
            assert_eq!(mode_of(key_path.parent().unwrap()).unwrap(), DIR_MODE);
        }
    }

    #[tokio::test]
    async fn env_key_takes_precedence_and_a_lost_key_is_never_replaced() {
        let fx = fixture();
        let env_key = MasterKey::generate().unwrap().to_base64().to_string();
        let store = Store::new(fx.paths.clone(), EnvKey::from_value(Some(env_key.clone())));
        store
            .set("A", &VALUE.into(), Some(vec!["w".into()]), None)
            .await
            .unwrap();
        let status = store.status().await.unwrap();
        assert_eq!(status.key_source, KeySourceKind::Env);
        assert!(status.key_path.is_none());
        assert!(
            !fx.paths.key_dir.as_ref().unwrap().exists(),
            "env key never writes a key file"
        );

        // Without the env key there is no key file: refuse, do not regenerate.
        let without = Store::new(fx.paths.clone(), EnvKey::default());
        let error = without.resolve("A", Some("w")).await.unwrap_err();
        assert_eq!(error.code, codes::KEY_UNAVAILABLE);
        let error = without
            .set("B", &"x-value-123456".into(), None, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, codes::KEY_UNAVAILABLE);
        // A different env key is a mismatch, not a decryption failure.
        let other = MasterKey::generate().unwrap().to_base64().to_string();
        let wrong = Store::new(fx.paths.clone(), EnvKey::from_value(Some(other)));
        assert_eq!(
            wrong.resolve("A", Some("w")).await.unwrap_err().code,
            codes::KEY_MISMATCH
        );
        let right = Store::new(fx.paths.clone(), EnvKey::from_value(Some(env_key)));
        assert_eq!(right.resolve("A", Some("w")).await.unwrap().expose(), VALUE);
    }

    #[tokio::test]
    async fn a_corrupt_vault_is_never_overwritten() {
        let fx = fixture();
        std::fs::create_dir_all(&fx.paths.data_dir).unwrap();
        std::fs::write(fx.paths.vault_path(), "{ not json").unwrap();
        let store = Store::new(fx.paths.clone(), EnvKey::default());
        assert_eq!(store.list().await.unwrap_err().code, codes::VAULT_ERROR);
        assert_eq!(
            store
                .set("A", &VALUE.into(), None, None)
                .await
                .unwrap_err()
                .code,
            codes::VAULT_ERROR
        );
        assert_eq!(
            std::fs::read_to_string(fx.paths.vault_path()).unwrap(),
            "{ not json"
        );
    }

    #[tokio::test]
    async fn invalid_input_is_rejected_without_echo() {
        let fx = fixture();
        let store = Store::new(fx.paths.clone(), EnvKey::default());
        for (name, value) in [
            ("bad name", "v-1234567890"),
            ("A", "   "),
            ("III_SECRETS_KEY", "abcdefgh12345"),
        ] {
            let error = store
                .set(name, &value.into(), None, None)
                .await
                .unwrap_err();
            assert_eq!(error.code, codes::INVALID_REQUEST);
            assert!(!error.message.contains("1234567890"));
        }
        let huge = "x".repeat(MAX_VALUE_BYTES + 1);
        assert!(store
            .set("A", &huge.as_str().into(), None, None)
            .await
            .is_err());
        assert_eq!(
            store.access("MISSING", vec![]).await.unwrap_err().code,
            codes::SECRET_NOT_FOUND
        );
    }

    #[tokio::test]
    async fn compare_reports_matches_without_values() {
        let fx = fixture();
        let store = Store::new(fx.paths.clone(), EnvKey::default());
        store.set("A", &VALUE.into(), None, None).await.unwrap();
        let found = vec![
            Found {
                name: "A".into(),
                kind: SourceKind::Dotenv,
                location: "/p/.env".into(),
                value: VALUE.into(),
            },
            Found {
                name: "A".into(),
                kind: SourceKind::ProcessEnv,
                location: "env".into(),
                value: "sk-different-value-0000".into(),
            },
            Found {
                name: "B".into(),
                kind: SourceKind::LoginShell,
                location: "/bin/zsh".into(),
                value: "sk-b-value-1111".into(),
            },
        ];
        let names = vec!["A".to_owned(), "B".to_owned(), "C".to_owned()];
        let response = store.compare(&names, &found).await.unwrap();
        let json = serde_json::to_string(&response).unwrap();
        assert!(
            !json.contains("abcdefghijklmnop") && !json.contains("different-value"),
            "{json}"
        );
        let a = &response.results[0];
        assert!(a.stored);
        assert_eq!(a.stored_hint.as_deref(), Some("sk-ant…9f2c"));
        assert!(a.sources[0].matches_stored);
        assert!(!a.sources[1].matches_stored);
        let b = &response.results[1];
        assert!(!b.stored && b.stored_hint.is_none());
        assert_eq!(b.sources.len(), 1);
        assert!(!b.sources[0].matches_stored);
        assert!(response.results[2].sources.is_empty());
    }

    fn dotenv(fx: &Fixture) -> PathBuf {
        let path = fx.paths.dotenv.clone().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        path
    }

    #[tokio::test]
    async fn env_store_shares_a_variable_and_reads_it_live() {
        let fx = fixture();
        let path = dotenv(&fx);
        std::fs::write(&path, format!("ANTHROPIC_API_KEY={VALUE}\n")).unwrap();
        let store = Store::new(fx.paths.clone(), EnvKey::default()).without_process_env();
        let (meta, event) = store
            .set_env(
                "ANTHROPIC_API_KEY",
                None,
                Some(vec!["llm-router".into()]),
                None,
            )
            .await
            .unwrap();
        assert_eq!(meta.reference, "env://ANTHROPIC_API_KEY");
        assert_eq!(meta.store, StoreKind::Env);
        assert_eq!(meta.hint, "sk-ant…9f2c");
        assert_eq!(meta.location.as_deref(), Some(path.to_str().unwrap()));
        assert!(meta.fingerprint.is_none());
        assert_eq!(event.reference, "env://ANTHROPIC_API_KEY");
        assert_eq!(event.action, ChangeAction::Created);

        let value = store
            .resolve_env("ANTHROPIC_API_KEY", Some("llm-router"))
            .await
            .unwrap();
        assert_eq!(value.expose(), VALUE);
        assert_eq!(
            store
                .resolve_env("ANTHROPIC_API_KEY", Some("harness"))
                .await
                .unwrap_err()
                .code,
            codes::SECRET_FORBIDDEN
        );
        // An edit to .env is what the next resolution returns.
        std::fs::write(&path, "ANTHROPIC_API_KEY=sk-ant-edited-by-hand-0000\n").unwrap();
        assert_eq!(
            store
                .resolve_env("ANTHROPIC_API_KEY", Some("llm-router"))
                .await
                .unwrap()
                .expose(),
            "sk-ant-edited-by-hand-0000"
        );
        let meta = store.get_env("ANTHROPIC_API_KEY").await.unwrap().unwrap();
        assert_eq!(meta.last_resolved_by.as_deref(), Some("llm-router"));

        // The vault keeps who may read it, never the value.
        let on_disk = std::fs::read_to_string(fx.paths.vault_path()).unwrap();
        assert!(on_disk.contains("\"env\""), "{on_disk}");
        assert!(!on_disk.contains("edited-by-hand") && !on_disk.contains("abcdefghij"));
        // Both stores list, each with its own reference.
        store.set("OTHER", &VALUE.into(), None, None).await.unwrap();
        let refs: Vec<String> = store
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .map(|meta| meta.reference)
            .collect();
        assert_eq!(refs, vec!["secret://OTHER", "env://ANTHROPIC_API_KEY"]);
    }

    #[tokio::test]
    async fn env_store_writes_dotenv_and_reports_edits_made_elsewhere() {
        let fx = fixture();
        let path = dotenv(&fx);
        std::fs::write(&path, "# Provider keys\n# OPENAI_API_KEY=\nA=1\n").unwrap();
        let store = Store::new(fx.paths.clone(), EnvKey::default()).without_process_env();
        store.open().await.unwrap();
        let (meta, event) = store
            .set_env(
                "OPENAI_API_KEY",
                Some(&"sk-proj-first-0000".into()),
                Some(vec!["llm-router".into()]),
                None,
            )
            .await
            .unwrap();
        assert_eq!(event.action, ChangeAction::Created);
        assert_eq!(meta.hint, "sk-p…0000");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Provider keys\nOPENAI_API_KEY=sk-proj-first-0000\nA=1\n"
        );
        // Its own write is not reported again.
        assert!(store.env_changes().await.is_empty());

        let (_, event) = store
            .set_env(
                "OPENAI_API_KEY",
                Some(&"sk-proj-second-1111".into()),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(event.action, ChangeAction::Rotated);
        let (meta, event) = store
            .set_env("OPENAI_API_KEY", None, Some(vec!["judge".into()]), None)
            .await
            .unwrap();
        assert_eq!(event.action, ChangeAction::AccessChanged);
        assert_eq!(meta.consumers, vec!["judge"]);

        let edit = |text: &str| std::fs::write(&path, text).unwrap();
        edit("OPENAI_API_KEY=sk-proj-by-hand-2222\n");
        let events = store.env_changes().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reference, "env://OPENAI_API_KEY");
        assert_eq!(events[0].action, ChangeAction::Rotated);
        assert!(store.env_changes().await.is_empty(), "unchanged file");
        edit("# removed\n");
        assert_eq!(store.env_changes().await[0].action, ChangeAction::Deleted);
        edit("OPENAI_API_KEY=sk-proj-is-back-3333\n");
        assert_eq!(store.env_changes().await[0].action, ChangeAction::Created);
    }

    #[tokio::test]
    async fn a_new_env_file_is_compared_with_the_old_one() {
        let fx = fixture();
        let path = dotenv(&fx);
        std::fs::write(
            &path,
            "A=same-value-0000\nB=dev-value-1111\nC=only-in-dev-2222\n",
        )
        .unwrap();
        let store = Store::new(fx.paths.clone(), EnvKey::default()).without_process_env();
        for name in ["A", "B", "C", "D"] {
            store
                .set_env(name, None, Some(vec!["w".into()]), None)
                .await
                .unwrap();
        }
        let staging = path.with_file_name(".env.staging");
        std::fs::write(
            &staging,
            "A=same-value-0000\nB=staging-value-3333\nD=only-in-staging\n",
        )
        .unwrap();
        assert!(
            store
                .reconfigure(StorePaths {
                    dotenv: Some(staging.clone()),
                    ..fx.paths.clone()
                })
                .await
        );
        let changes: Vec<(String, ChangeAction)> = store
            .env_changes()
            .await
            .into_iter()
            .map(|event| (event.name, event.action))
            .collect();
        assert_eq!(
            changes,
            vec![
                ("B".to_owned(), ChangeAction::Rotated),
                ("C".to_owned(), ChangeAction::Deleted),
                ("D".to_owned(), ChangeAction::Created),
            ]
        );
        assert_eq!(
            store.resolve_env("B", Some("w")).await.unwrap().expose(),
            "staging-value-3333"
        );
        // Writes go to the configured file, never the old one.
        store
            .set_env("E", Some(&"written-to-staging".into()), None, None)
            .await
            .unwrap();
        assert!(std::fs::read_to_string(&staging)
            .unwrap()
            .contains("E=written-to-staging"));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("E="));
        assert_eq!(
            store.status().await.unwrap().env_file.as_deref(),
            staging.to_str()
        );
    }

    #[tokio::test]
    async fn env_references_need_a_grant_and_a_value() {
        let fx = fixture();
        let path = dotenv(&fx);
        let store = Store::new(fx.paths.clone(), EnvKey::default()).without_process_env();
        // Not shared: refused without echoing the name.
        let error = store
            .resolve_env("sk-live-pasted", Some("w"))
            .await
            .unwrap_err();
        assert_eq!(error.code, codes::SECRET_FORBIDDEN);
        assert!(!error.message.contains("sk-live-pasted"), "{error}");
        // Shared with nobody yet.
        store.set_env("K", None, None, None).await.unwrap();
        assert_eq!(
            store.resolve_env("K", Some("w")).await.unwrap_err().code,
            codes::SECRET_FORBIDDEN
        );
        // Shared but unset.
        let (meta, _) = store
            .set_env("K", None, Some(vec!["w".into()]), None)
            .await
            .unwrap();
        assert_eq!(meta.hint, "");
        assert!(meta.location.is_none());
        let error = store.resolve_env("K", Some("w")).await.unwrap_err();
        assert_eq!(error.code, codes::SECRET_NOT_FOUND);
        assert!(error.message.contains(path.to_str().unwrap()), "{error}");
        // A value .env cannot hold is refused before anything is written.
        assert_eq!(
            store
                .set_env("K", Some(&"two\nlines-value".into()), None, None)
                .await
                .unwrap_err()
                .code,
            codes::INVALID_REQUEST
        );
        assert!(!path.exists());
        // Unsharing keeps the variable and restores the original vault layout.
        std::fs::write(&path, "K=still-here-value\n").unwrap();
        assert!(store.delete_env("K").await.unwrap().is_some());
        assert!(store.delete_env("K").await.unwrap().is_none());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "K=still-here-value\n"
        );
        let on_disk = std::fs::read_to_string(fx.paths.vault_path()).unwrap();
        assert!(!on_disk.contains("\"env\""), "{on_disk}");
        assert_eq!(store.status().await.unwrap().env_count, 0);
    }

    #[tokio::test]
    async fn reconfigure_switches_vaults() {
        let first = fixture();
        let second = fixture();
        let store = Store::new(first.paths.clone(), EnvKey::default());
        store.set("A", &VALUE.into(), None, None).await.unwrap();
        assert!(!store.reconfigure(first.paths.clone()).await);
        assert!(store.reconfigure(second.paths.clone()).await);
        assert!(store.list().await.unwrap().is_empty());
        assert_eq!(store.vault_path().await, second.paths.vault_path());
    }
}
