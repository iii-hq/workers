//! `<handle>/<name>[@<version|tag|range>]` — how callers name a kit.

use std::fmt;

/// What follows the `@`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionRef {
    /// An exact semver (`1.3.0`).
    Exact(String),
    /// A release tag (`latest`, `next`).
    Tag(String),
    /// A semver range (`^1.3`, `~1.2.0`, `>=1, <2`).
    Range(String),
}

impl VersionRef {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("the version after '@' is empty".into());
        }
        if semver::Version::parse(raw.trim_start_matches('v')).is_ok() {
            return Ok(Self::Exact(raw.trim_start_matches('v').to_string()));
        }
        let looks_like_range = raw
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '^' | '~' | '>' | '<' | '=' | '*'));
        if looks_like_range {
            semver::VersionReq::parse(raw)
                .map_err(|e| format!("{raw:?} is neither a version nor a valid range: {e}"))?;
            return Ok(Self::Range(raw.to_string()));
        }
        if raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Ok(Self::Tag(raw.to_string()));
        }
        Err(format!(
            "{raw:?} is not a version, a release tag or a range"
        ))
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Exact(v) | Self::Tag(v) | Self::Range(v) => v,
        }
    }

    /// Does `version` satisfy this ref? Tags never decide on their own.
    pub fn matches(&self, version: &str) -> Option<bool> {
        let v = semver::Version::parse(version).ok()?;
        match self {
            Self::Exact(e) => Some(semver::Version::parse(e).ok()? == v),
            Self::Range(r) => Some(semver::VersionReq::parse(r).ok()?.matches(&v)),
            Self::Tag(_) => None,
        }
    }
}

impl fmt::Display for VersionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitRef {
    pub handle: String,
    pub name: String,
    /// `None` when the caller gave no `@…`.
    pub version: Option<VersionRef>,
}

impl KitRef {
    pub fn id(&self) -> String {
        format!("{}/{}", self.handle, self.name)
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        let (id, version) = match raw.split_once('@') {
            Some((id, version)) => (id, Some(VersionRef::parse(version)?)),
            None => (raw, None),
        };
        let (handle, name) = id
            .split_once('/')
            .ok_or_else(|| format!("{raw:?} is not <author>/<kit>"))?;
        validate_handle(handle)?;
        validate_kit_name(name)?;
        Ok(Self {
            handle: handle.to_string(),
            name: name.to_string(),
            version,
        })
    }
}

fn segment_ok(s: &str, max: usize) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.len() <= max
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn validate_handle(handle: &str) -> Result<(), String> {
    if segment_ok(handle, 39) {
        Ok(())
    } else {
        Err(format!(
            "author handle {handle:?} must match ^[a-z0-9][a-z0-9-]{{0,38}}$"
        ))
    }
}

pub fn validate_kit_name(name: &str) -> Result<(), String> {
    if segment_ok(name, 64) {
        Ok(())
    } else {
        Err(format!(
            "kit name {name:?} must match ^[a-z0-9][a-z0-9-]{{0,63}}$"
        ))
    }
}

/// Validate a full kit id `<handle>/<name>` (no version).
pub fn validate_kit_id(kit: &str) -> Result<(), String> {
    let parsed = KitRef::parse(kit)?;
    if parsed.version.is_some() {
        return Err(format!("{kit:?} must not carry a version here"));
    }
    Ok(())
}

/// Is `to` a major jump from `from` (`1.x` → `2.x`, or `0.1` → `0.2`)?
pub fn is_major_jump(from: &str, to: &str) -> bool {
    match (semver::Version::parse(from), semver::Version::parse(to)) {
        (Ok(a), Ok(b)) => {
            if a.major == 0 && b.major == 0 {
                a.minor != b.minor
            } else {
                a.major != b.major
            }
        }
        _ => false,
    }
}

/// Does `version` satisfy the semver `range`? `None` when either is not semver.
pub fn satisfies(range: &str, version: &str) -> Option<bool> {
    let req = semver::VersionReq::parse(range).ok()?;
    let v = semver::Version::parse(version).ok()?;
    // A prerelease is compared as the release it leads to (what Compose does).
    Some(req.matches(&semver::Version::new(v.major, v.minor, v.patch)))
}

/// Classify the semver jump `from` → `to`: `major`, `minor`, `patch` or `prerelease`.
pub fn bump_kind(from: &str, to: &str) -> Option<&'static str> {
    let a = semver::Version::parse(from).ok()?;
    let b = semver::Version::parse(to).ok()?;
    Some(if is_major_jump(from, to) {
        "major"
    } else if a.minor != b.minor || a.major != b.major {
        "minor"
    } else if a.patch != b.patch {
        "patch"
    } else {
        "prerelease"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_tag_exact_and_range_refs() {
        let r = KitRef::parse("acme/kanban-team").unwrap();
        assert_eq!(r.id(), "acme/kanban-team");
        assert_eq!(r.version, None);
        assert_eq!(
            KitRef::parse("acme/kanban-team@1.3.0").unwrap().version,
            Some(VersionRef::Exact("1.3.0".into()))
        );
        assert_eq!(
            KitRef::parse("acme/kanban-team@next").unwrap().version,
            Some(VersionRef::Tag("next".into()))
        );
        assert_eq!(
            KitRef::parse("acme/kanban-team@^1.3").unwrap().version,
            Some(VersionRef::Range("^1.3".into()))
        );
        assert_eq!(
            KitRef::parse("acme/kanban-team@1.3").unwrap().version,
            Some(VersionRef::Range("1.3".into()))
        );
    }

    #[test]
    fn rejects_malformed_refs() {
        assert!(KitRef::parse("kanban-team").is_err());
        assert!(KitRef::parse("Acme/kanban").is_err());
        assert!(KitRef::parse("acme/kanban_team").is_err());
        assert!(KitRef::parse("acme/kanban@").is_err());
        assert!(KitRef::parse("acme/kanban@^x.y").is_err());
        assert!(KitRef::parse("a/b/c").is_err());
    }

    #[test]
    fn major_jumps_follow_semver_zero_rules() {
        assert!(is_major_jump("1.3.0", "2.0.0"));
        assert!(!is_major_jump("1.3.0", "1.4.0"));
        assert!(is_major_jump("0.1.0", "0.2.0"));
        assert!(!is_major_jump("0.1.0", "0.1.5"));
        assert_eq!(bump_kind("1.0.0", "1.1.0"), Some("minor"));
        assert_eq!(bump_kind("1.0.0", "1.0.1"), Some("patch"));
    }

    #[test]
    fn satisfies_treats_prereleases_as_their_release() {
        assert_eq!(satisfies("^1.4", "1.6.1"), Some(true));
        assert_eq!(satisfies("^1.6", "1.5.2"), Some(false));
        assert_eq!(satisfies("^1.0", "1.1.0-rc.1"), Some(true));
        assert_eq!(satisfies("next", "1.0.0"), None);
    }
}
