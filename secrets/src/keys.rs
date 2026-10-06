//! Where the master key comes from, in order:
//!
//! 1. `III_SECRETS_KEY` (base64, 32 bytes), captured once at startup and then
//!    removed from the process environment so no child (the login shell that
//!    `secrets::detect` runs) inherits it;
//! 2. the key file: config `key_file`, else
//!    `${XDG_CONFIG_HOME:-~/.config}/iii/secrets/<vault_id>.key`, created
//!    (0600, directory 0700) the first time a value is sealed.
//!
//! Either way the key must match the vault header's `key_check`, so a wrong
//! key is refused before it seals anything, and a missing key file is never
//! silently replaced once the vault holds values.
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::crypto::MasterKey;
use crate::error::{codes, SecretsError};
use crate::fsutil;
use crate::secret::REDACTED;

pub const KEY_ENV: &str = "III_SECRETS_KEY";
const KEY_FILE_EXTENSION: &str = "key";

/// The `III_SECRETS_KEY` value captured at startup.
#[derive(Clone, Default)]
pub struct EnvKey(Option<Zeroizing<String>>);

impl fmt::Debug for EnvKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(_) => write!(f, "EnvKey({REDACTED})"),
            None => f.write_str("EnvKey(unset)"),
        }
    }
}

impl EnvKey {
    /// Read `III_SECRETS_KEY` and remove it from this process's environment.
    /// Call before any thread is spawned: removing a variable while another
    /// thread reads the environment is unsound on some platforms.
    pub fn take_from_process() -> Self {
        let value = std::env::var(KEY_ENV).ok();
        std::env::remove_var(KEY_ENV);
        Self::from_value(value)
    }

    /// Blank values count as unset.
    pub fn from_value(value: Option<String>) -> Self {
        Self(
            value
                .filter(|value| !value.trim().is_empty())
                .map(Zeroizing::new),
        )
    }

    pub fn is_set(&self) -> bool {
        self.0.is_some()
    }
}

/// Where the key in use came from (`secrets::status.key_source`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySource {
    Env,
    File(PathBuf),
}

/// `${XDG_CONFIG_HOME:-~/.config}/iii/secrets`, or `None` without a home.
pub fn default_key_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()));
    default_key_dir_with(std::env::var_os("XDG_CONFIG_HOME"), home)
}

pub fn default_key_dir_with(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    // The XDG spec ignores relative values.
    let base = xdg_config_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("iii").join("secrets"))
}

/// The key file for `vault_id`: the configured override, else
/// `<key_dir>/<vault_id>.key`.
pub fn key_file_path(
    key_file: Option<&Path>,
    key_dir: Option<&Path>,
    vault_id: &str,
) -> Option<PathBuf> {
    key_file
        .map(Path::to_path_buf)
        .or_else(|| key_dir.map(|dir| dir.join(format!("{vault_id}.{KEY_FILE_EXTENSION}"))))
}

/// What loading needs to know about the vault and the project.
pub struct KeyRequest<'a> {
    pub env: &'a EnvKey,
    /// `None` when there is no home directory and no `key_file` override.
    pub file: Option<&'a Path>,
    /// The vault header's `key_check`, once any key has been used with it.
    pub key_check: Option<&'a str>,
    /// Generate the key file when it is missing. Only a write to a vault that
    /// has never sealed anything may create one.
    pub create: bool,
    /// The Compose project directory (`III_COMPOSE_DIR`): the key file must
    /// not live inside it, where it could be committed next to the vault.
    pub project_dir: Option<&'a Path>,
}

pub struct LoadedKey {
    pub key: MasterKey,
    pub source: KeySource,
    pub created: bool,
}

pub fn load_master_key(request: KeyRequest<'_>) -> Result<LoadedKey, SecretsError> {
    if let Some(text) = request.env.0.as_ref() {
        let key = MasterKey::from_base64(text).ok_or_else(|| {
            SecretsError::key(format!("{KEY_ENV} must be base64 for exactly 32 bytes"))
        })?;
        verify(&key, request.key_check, KEY_ENV)?;
        return Ok(LoadedKey {
            key,
            source: KeySource::Env,
            created: false,
        });
    }
    let file = request.file.ok_or_else(|| {
        SecretsError::key(format!(
            "no location for the key file: set HOME or XDG_CONFIG_HOME, the `key_file` setting, or {KEY_ENV}"
        ))
    })?;
    if let Some(project) = request.project_dir {
        if is_within(file, project) {
            return Err(SecretsError::key(format!(
                "key file {} is inside the project directory; point `key_file` outside it or set {KEY_ENV}",
                file.display()
            )));
        }
    }
    for _ in 0..2 {
        match std::fs::read_to_string(file) {
            Ok(text) => {
                let text = Zeroizing::new(text);
                let key = MasterKey::from_base64(&text).ok_or_else(|| {
                    SecretsError::key(format!(
                        "key file {} does not hold a base64 32-byte key",
                        file.display()
                    ))
                })?;
                if let Err(error) = fsutil::tighten(file, fsutil::FILE_MODE) {
                    tracing::warn!(path = %file.display(), error = %error, "could not narrow key file permissions");
                }
                verify(&key, request.key_check, &file.display().to_string())?;
                return Ok(LoadedKey {
                    key,
                    source: KeySource::File(file.to_path_buf()),
                    created: false,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if request.key_check.is_some() {
                    return Err(SecretsError::key(format!(
                        "key file {} is missing; restore it or set {KEY_ENV}. Values in this vault cannot be decrypted without it",
                        file.display()
                    )));
                }
                if !request.create {
                    return Err(SecretsError::key(format!(
                        "no master key yet: key file {} is created when the first secret is stored",
                        file.display()
                    )));
                }
                let key = MasterKey::generate()?;
                let mut contents = key.to_base64();
                contents.push('\n');
                match fsutil::create_private_exclusive(file, contents.as_bytes()) {
                    Ok(true) => {
                        return Ok(LoadedKey {
                            key,
                            source: KeySource::File(file.to_path_buf()),
                            created: true,
                        })
                    }
                    // Someone else created it first: read theirs.
                    Ok(false) => continue,
                    Err(error) => {
                        return Err(SecretsError::key(format!(
                            "key file {} could not be created: {}",
                            file.display(),
                            error.kind()
                        )))
                    }
                }
            }
            Err(error) => {
                return Err(SecretsError::key(format!(
                    "key file {} could not be read: {}",
                    file.display(),
                    error.kind()
                )))
            }
        }
    }
    Err(SecretsError::key(format!(
        "key file {} changed while it was being created",
        file.display()
    )))
}

fn verify(key: &MasterKey, key_check: Option<&str>, origin: &str) -> Result<(), SecretsError> {
    match key_check {
        Some(expected) if key.key_check() != expected => Err(SecretsError::new(
            codes::KEY_MISMATCH,
            format!("the master key from {origin} does not match this vault"),
        )),
        _ => Ok(()),
    }
}

/// Whether `path` lies under `dir`, comparing canonical forms where the
/// filesystem allows (the key file itself may not exist yet).
pub fn is_within(path: &Path, dir: &Path) -> bool {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let path = match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map(|parent| parent.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    };
    path.starts_with(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(env: &'a EnvKey, file: &'a Path) -> KeyRequest<'a> {
        KeyRequest {
            env,
            file: Some(file),
            key_check: None,
            create: true,
            project_dir: None,
        }
    }

    #[test]
    fn env_key_wins_over_the_key_file() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("keys/vault.key");
        let file_key = load_master_key(request(&EnvKey::default(), &file)).unwrap();
        assert!(file_key.created);
        assert_eq!(file_key.source, KeySource::File(file.clone()));

        let env_secret = MasterKey::generate().unwrap();
        let env = EnvKey::from_value(Some(env_secret.to_base64().to_string()));
        let loaded = load_master_key(request(&env, &file)).unwrap();
        assert_eq!(loaded.source, KeySource::Env);
        assert_eq!(loaded.key.key_check(), env_secret.key_check());
        assert_ne!(loaded.key.key_check(), file_key.key.key_check());
    }

    #[test]
    fn the_key_file_is_created_once_then_reused() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("iii/secrets/v.key");
        let env = EnvKey::default();
        let first = load_master_key(request(&env, &file)).unwrap();
        let second = load_master_key(request(&env, &file)).unwrap();
        assert!(first.created && !second.created);
        assert_eq!(first.key.key_check(), second.key.key_check());
        #[cfg(unix)]
        {
            assert_eq!(fsutil::mode_of(&file).unwrap(), fsutil::FILE_MODE);
            assert_eq!(
                fsutil::mode_of(file.parent().unwrap()).unwrap(),
                fsutil::DIR_MODE
            );
        }
    }

    #[test]
    fn blank_env_counts_as_unset_and_malformed_env_is_refused() {
        assert!(!EnvKey::from_value(Some("  ".into())).is_set());
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("v.key");
        let env = EnvKey::from_value(Some("dG9vIHNob3J0".into()));
        let error = load_master_key(request(&env, &file)).err().unwrap();
        assert_eq!(error.code, codes::KEY_UNAVAILABLE);
        assert!(!error.message.contains("dG9vIHNob3J0"));
        assert!(
            !file.exists(),
            "a bad env key never falls back to creating a file"
        );
    }

    #[test]
    fn a_missing_key_is_never_regenerated_for_a_used_vault() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("v.key");
        let env = EnvKey::default();
        let mut used = request(&env, &file);
        used.key_check = Some("0123456789abcdef");
        let error = load_master_key(used).err().unwrap();
        assert_eq!(error.code, codes::KEY_UNAVAILABLE);
        assert!(!file.exists());
        let mut read_only = request(&env, &file);
        read_only.create = false;
        assert!(load_master_key(read_only).is_err());
        assert!(!file.exists());
    }

    #[test]
    fn a_key_that_does_not_match_the_vault_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("v.key");
        let env = EnvKey::default();
        let original = load_master_key(request(&env, &file)).unwrap();
        let check = original.key.key_check();
        let other =
            EnvKey::from_value(Some(MasterKey::generate().unwrap().to_base64().to_string()));
        let mut mismatched = request(&other, &file);
        mismatched.key_check = Some(&check);
        assert_eq!(
            load_master_key(mismatched).err().unwrap().code,
            codes::KEY_MISMATCH
        );
        let mut matching = request(&env, &file);
        matching.key_check = Some(&check);
        assert!(load_master_key(matching).is_ok());
    }

    #[test]
    fn the_key_file_may_not_live_in_the_project() {
        let project = tempfile::tempdir().unwrap();
        let file = project.path().join("config/secrets.key");
        let env = EnvKey::default();
        let mut inside = request(&env, &file);
        inside.project_dir = Some(project.path());
        assert_eq!(
            load_master_key(inside).err().unwrap().code,
            codes::KEY_UNAVAILABLE
        );
        assert!(!file.exists());
    }

    #[test]
    fn default_key_dir_follows_xdg_then_home() {
        assert_eq!(
            default_key_dir_with(Some("/xdg".into()), Some("/home/u".into())),
            Some(PathBuf::from("/xdg/iii/secrets"))
        );
        assert_eq!(
            default_key_dir_with(Some("relative".into()), Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.config/iii/secrets"))
        );
        assert_eq!(
            default_key_dir_with(None, Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.config/iii/secrets"))
        );
        assert_eq!(default_key_dir_with(None, None), None);
        assert_eq!(
            key_file_path(None, Some(Path::new("/k")), "abc"),
            Some(PathBuf::from("/k/abc.key"))
        );
        assert_eq!(
            key_file_path(
                Some(Path::new("/elsewhere/x.key")),
                Some(Path::new("/k")),
                "abc"
            ),
            Some(PathBuf::from("/elsewhere/x.key"))
        );
    }

    #[test]
    fn debug_never_prints_the_env_key() {
        let env = EnvKey::from_value(Some("c2VjcmV0LWtleS1tYXRlcmlhbA==".into()));
        let printed = format!("{env:?}");
        assert!(!printed.contains("c2VjcmV0"), "{printed}");
    }
}
