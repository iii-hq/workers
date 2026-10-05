//! The vault in memory: every operation runs under one lock, writes go to a
//! copy that replaces the in-memory vault only after it is on disk.
use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{SecondsFormat, Utc};
use tokio::sync::Mutex;

use crate::access::is_authorized;
use crate::api::{
    DetectResponse, DetectResult, DetectedSource, KeySourceKind, SecretMeta, StatusResponse,
};
use crate::config::SecretsConfig;
use crate::crypto::MasterKey;
use crate::detect::Found;
use crate::error::{codes, SecretsError};
use crate::events::{ChangeAction, ChangedEvent};
use crate::keys::{self, EnvKey, KeyRequest};
use crate::names::{self, mask, normalize_consumers, validate_name};
use crate::secret::SecretString;
use crate::vault::{SecretRecord, Vault, VAULT_FILE};

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
    state: Mutex<State>,
}

struct State {
    paths: StorePaths,
    /// Loaded on first use; `None` again after a reconfiguration.
    vault: Option<Vault>,
    key: Option<MasterKey>,
}

impl State {
    fn new(paths: StorePaths) -> Self {
        Self {
            paths,
            vault: None,
            key: None,
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
            state: Mutex::new(State::new(paths)),
        }
    }

    /// Load (or create) the vault now; returns how many secrets it holds.
    pub async fn open(&self) -> Result<usize, SecretsError> {
        Ok(self.state.lock().await.vault()?.secrets.len())
    }

    pub async fn vault_path(&self) -> PathBuf {
        self.state.lock().await.paths.vault_path()
    }

    /// Point the store at new locations. The cached vault and key are
    /// dropped (the key is zeroized) and re-read on next use.
    pub async fn reconfigure(&self, paths: StorePaths) -> bool {
        let mut state = self.state.lock().await;
        if state.paths == paths {
            return false;
        }
        *state = State::new(paths);
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
            Some(meta.fingerprint.clone()),
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
            return Err(forbidden(name, caller, record.consumers.is_empty()));
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
        Ok(StatusResponse {
            vault_path: state.paths.vault_path().display().to_string(),
            key_source,
            key_path: key_path.map(|path| path.display().to_string()),
            count,
            version,
        })
    }
}

fn forbidden(name: &str, caller: Option<&str>, no_consumers: bool) -> SecretsError {
    let reference = names::reference_for(name);
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
        assert_eq!(
            event.fingerprint.as_deref(),
            Some(meta.fingerprint.as_str())
        );

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
