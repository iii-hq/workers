//! Tenants: the unit of isolation when one Mac serves several consumers.
//!
//! A tenant owns a private CoreSimulator device set
//! (`<data_dir>/tenants/<tenant>/devices`) and a media folder
//! (`<data_dir>/tenants/<tenant>/media/<udid>/`). Every device operation runs
//! against the caller's set, so another tenant's simulators are not merely
//! hidden, they do not exist for the call. The worker never decides who a
//! caller is: the tenant arrives in the payload, and on a shared deployment
//! the operator's `rbac-proxy` middleware stamps it from the authenticated
//! session (README › Multiple consumers).

use std::path::{Path, PathBuf};

use crate::config::{WorkerConfig, DEFAULT_TENANT};
use crate::simctl::{is_udid, DeviceSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    pub name: String,
    pub set: DeviceSet,
    /// `<data_dir>/tenants/<tenant>`.
    pub root: PathBuf,
}

/// Lowercase slug, 1-63 chars: safe as a path segment and a stream key.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn resolve(
    cfg: &WorkerConfig,
    developer_dir: &Path,
    requested: Option<&str>,
) -> Result<Tenant, String> {
    let name = match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => name,
        None if cfg.require_tenant => {
            return Err("tenant is required (the worker is configured with require_tenant)".into())
        }
        None => DEFAULT_TENANT,
    };
    if !valid_name(name) {
        return Err(format!(
            "invalid tenant '{name}': use 1-63 lowercase letters, digits, '-' or '_'"
        ));
    }
    let root = cfg.data_root().join("tenants").join(name);
    let path = if name == DEFAULT_TENANT && cfg.share_system_devices {
        None
    } else {
        Some(root.join("devices"))
    };
    Ok(Tenant {
        name: name.to_string(),
        set: DeviceSet {
            path,
            developer_dir: developer_dir.to_path_buf(),
        },
        root,
    })
}

/// Every tenant with a folder under `<data_dir>/tenants`, plus `default`.
pub fn known(cfg: &WorkerConfig) -> Vec<String> {
    let mut names = vec![DEFAULT_TENANT.to_string()];
    if let Ok(entries) = std::fs::read_dir(cfg.data_root().join("tenants")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if valid_name(&name) && entry.path().is_dir() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

impl Tenant {
    pub fn media_dir(&self, udid: &str) -> PathBuf {
        self.root.join("media").join(udid)
    }

    /// Resolve a media name (`<udid>/<file>`) to its path, refusing anything
    /// that is not exactly one of this tenant's screenshots or recordings.
    pub fn media_path(&self, name: &str) -> Result<PathBuf, String> {
        let (udid, file) = name
            .split_once('/')
            .ok_or_else(|| format!("invalid media name '{name}'"))?;
        let well_formed = is_udid(udid)
            && (file.starts_with("screenshot-") || file.starts_with("recording-"))
            && (file.ends_with(".png") || file.ends_with(".mov"))
            && file
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
        if !well_formed || file.contains("..") {
            return Err(format!("invalid media name '{name}'"));
        }
        Ok(self.media_dir(udid).join(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WorkerConfig {
        WorkerConfig {
            data_dir: "/srv/sim".into(),
            ..WorkerConfig::default()
        }
    }

    #[test]
    fn missing_tenant_is_default_unless_required() {
        let t = resolve(&cfg(), Path::new("/Xcode"), None).unwrap();
        assert_eq!(t.name, "default");
        let strict = WorkerConfig {
            require_tenant: true,
            ..cfg()
        };
        assert!(resolve(&strict, Path::new("/Xcode"), None).is_err());
        assert!(resolve(&strict, Path::new("/Xcode"), Some("  ")).is_err());
    }

    #[test]
    fn default_tenant_shares_the_system_set_only_when_configured() {
        let t = resolve(&cfg(), Path::new("/Xcode"), Some("default")).unwrap();
        assert_eq!(t.set.path, None);
        let private = WorkerConfig {
            share_system_devices: false,
            ..cfg()
        };
        let t = resolve(&private, Path::new("/Xcode"), Some("default")).unwrap();
        assert_eq!(
            t.set.path,
            Some(PathBuf::from("/srv/sim/tenants/default/devices"))
        );
    }

    #[test]
    fn tenants_get_private_sets() {
        let t = resolve(&cfg(), Path::new("/Xcode"), Some("acme")).unwrap();
        assert_eq!(
            t.set.path,
            Some(PathBuf::from("/srv/sim/tenants/acme/devices"))
        );
    }

    #[test]
    fn tenant_names_cannot_escape_the_data_dir() {
        for bad in ["../x", "a/b", "ACME", ".hidden", "-x", &"a".repeat(64)] {
            assert!(
                resolve(&cfg(), Path::new("/Xcode"), Some(bad)).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn media_names_are_confined_to_the_tenant() {
        let t = resolve(&cfg(), Path::new("/Xcode"), Some("acme")).unwrap();
        let udid = "5FEFD8EB-AB2D-4B6A-A5AB-6604312D1993";
        assert_eq!(
            t.media_path(&format!("{udid}/screenshot-20260927T151500123Z.png"))
                .unwrap(),
            PathBuf::from(format!(
                "/srv/sim/tenants/acme/media/{udid}/screenshot-20260927T151500123Z.png"
            ))
        );
        for bad in [
            format!("{udid}/../../other/media/x.png"),
            format!("{udid}/notes.txt"),
            "../acme/screenshot-1.png".to_string(),
            format!("{udid}/screenshot-1.png/x"),
        ] {
            assert!(t.media_path(&bad).is_err(), "{bad}");
        }
    }
}
