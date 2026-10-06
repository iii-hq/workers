//! The vault file: `<data_dir>/vault.json`, a header plus one sealed record
//! per secret. Nothing in it is plaintext: values are XChaCha20-Poly1305
//! ciphertexts, and the hint and fingerprint reveal at most a masked prefix
//! and suffix and a keyed hash. Environment variables shared through the
//! `env` store add a grant each (who may read it), never a value.
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::api::{SecretMeta, StoreKind};
use crate::crypto::{self, Sealed, NONCE_LEN};
use crate::envstore::EnvValue;
use crate::error::SecretsError;
use crate::fsutil;
use crate::names::{is_valid_name, mask, reference_for};

pub const VAULT_FILE: &str = "vault.json";
pub const VAULT_VERSION: u32 = 1;
pub const CIPHER: &str = "xchacha20poly1305";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vault {
    pub version: u32,
    /// Random, not secret: names the default key file.
    pub vault_id: String,
    pub cipher: String,
    /// `MasterKey::key_check` of the key that sealed this vault's values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_check: Option<String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretRecord>,
    /// `env://NAME` grants. Omitted while empty, so a vault that never
    /// shared a variable keeps the original layout.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, EnvGrant>,
}

/// Who may resolve one environment variable through `env://NAME`. The value
/// stays in `.env` or the worker's environment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvGrant {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub consumers: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resolved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resolved_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecretRecord {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub consumers: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resolved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resolved_by: Option<String>,
    /// `names::mask` of the value.
    pub hint: String,
    /// `MasterKey::fingerprint` of the value.
    pub fingerprint: String,
    /// base64 24-byte nonce.
    pub nonce: String,
    /// base64 ciphertext + Poly1305 tag.
    pub ciphertext: String,
}

impl Default for Vault {
    fn default() -> Self {
        Self::new()
    }
}

impl Vault {
    pub fn new() -> Self {
        Self {
            version: VAULT_VERSION,
            vault_id: uuid::Uuid::new_v4().to_string(),
            cipher: CIPHER.to_owned(),
            key_check: None,
            secrets: BTreeMap::new(),
            env: BTreeMap::new(),
        }
    }

    /// `Ok(None)` when the file does not exist yet. A file that exists but
    /// cannot be parsed is an error, never an empty vault: the next write
    /// would otherwise destroy every stored value.
    pub fn load(path: &Path) -> Result<Option<Self>, SecretsError> {
        let raw = match std::fs::read(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(SecretsError::vault(format!(
                    "vault {} could not be read: {}",
                    path.display(),
                    error.kind()
                )))
            }
        };
        let vault: Self = serde_json::from_slice(&raw).map_err(|error| {
            SecretsError::vault(format!(
                "vault {} is not valid ({:?} error at line {}); fix or move it aside",
                path.display(),
                error.classify(),
                error.line()
            ))
        })?;
        vault.validate(path)?;
        if let Err(error) = fsutil::tighten(path, fsutil::FILE_MODE) {
            tracing::warn!(path = %path.display(), error = %error, "could not narrow vault permissions");
        }
        Ok(Some(vault))
    }

    pub fn save(&self, path: &Path) -> Result<(), SecretsError> {
        let mut json = serde_json::to_vec_pretty(self)
            .map_err(|_| SecretsError::vault("vault could not be serialized"))?;
        json.push(b'\n');
        fsutil::write_private_atomic(path, &json).map_err(|error| {
            SecretsError::vault(format!(
                "vault {} could not be written: {}",
                path.display(),
                error.kind()
            ))
        })
    }

    fn validate(&self, path: &Path) -> Result<(), SecretsError> {
        if self.version != VAULT_VERSION {
            return Err(SecretsError::vault(format!(
                "vault {} has unsupported version {} (this worker reads {VAULT_VERSION})",
                path.display(),
                self.version
            )));
        }
        if self.cipher != CIPHER {
            return Err(SecretsError::vault(format!(
                "vault {} uses unsupported cipher {}",
                path.display(),
                self.cipher
            )));
        }
        if self.vault_id.trim().is_empty() || self.vault_id.contains(['/', '\\']) {
            return Err(SecretsError::vault(format!(
                "vault {} has an invalid vault_id",
                path.display()
            )));
        }
        let names = self
            .secrets
            .iter()
            .map(|(key, record)| (key, &record.name))
            .chain(self.env.iter().map(|(key, grant)| (key, &grant.name)));
        for (key, name) in names {
            if key != name || !is_valid_name(key) {
                return Err(SecretsError::vault(format!(
                    "vault {} has a record whose name does not match its key",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

impl SecretRecord {
    pub fn sealed(&self) -> Result<Sealed, SecretsError> {
        let corrupt = || SecretsError::vault(format!("record `{}` is corrupt", self.name));
        let nonce: [u8; NONCE_LEN] = crypto::decode(&self.nonce)
            .ok_or_else(corrupt)?
            .try_into()
            .map_err(|_| corrupt())?;
        let ciphertext = crypto::decode(&self.ciphertext).ok_or_else(corrupt)?;
        Ok(Sealed { nonce, ciphertext })
    }

    pub fn set_sealed(&mut self, sealed: &Sealed) {
        self.nonce = crypto::encode(&sealed.nonce);
        self.ciphertext = crypto::encode(&sealed.ciphertext);
    }

    pub fn meta(&self) -> SecretMeta {
        SecretMeta {
            name: self.name.clone(),
            reference: reference_for(StoreKind::Vault, &self.name),
            store: StoreKind::Vault,
            description: self.description.clone(),
            consumers: self.consumers.clone(),
            hint: self.hint.clone(),
            fingerprint: Some(self.fingerprint.clone()),
            location: None,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            last_resolved_at: self.last_resolved_at.clone(),
            last_resolved_by: self.last_resolved_by.clone(),
        }
    }
}

impl EnvGrant {
    /// Metadata with the variable's current hint and location (`value` is
    /// `None` while it is not set).
    pub fn meta(&self, value: Option<&EnvValue>) -> SecretMeta {
        SecretMeta {
            name: self.name.clone(),
            reference: reference_for(StoreKind::Env, &self.name),
            store: StoreKind::Env,
            description: self.description.clone(),
            consumers: self.consumers.clone(),
            hint: value.map(|v| mask(v.value.expose())).unwrap_or_default(),
            fingerprint: None,
            location: value.map(|v| v.location.clone()),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            last_resolved_at: self.last_resolved_at.clone(),
            last_resolved_by: self.last_resolved_by.clone(),
        }
    }
}
