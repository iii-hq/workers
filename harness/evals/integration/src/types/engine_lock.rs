//! `engine.lock` — the pinned engine source the runner (and CI) builds. The
//! runner never downloads artifacts; it validates the four fields and records
//! them in `stack.json` (spec § Decisions / § CI and gate policy).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineLock {
    pub repository: String,
    pub revision: String,
    pub package: String,
    pub binary: String,
}

impl EngineLock {
    pub fn parse(contents: &str) -> anyhow::Result<Self> {
        let lock: EngineLock = toml::from_str(contents)?;
        lock.validate()?;
        Ok(lock)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.revision.len() != 40 || !self.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            anyhow::bail!(
                "engine.lock revision must be a 40-hex commit, got {:?}",
                self.revision
            );
        }
        for (field, value) in [
            ("repository", &self.repository),
            ("package", &self.package),
            ("binary", &self.binary),
        ] {
            if value.trim().is_empty() {
                anyhow::bail!("engine.lock {field} must not be empty");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_committed_shape() {
        let lock = EngineLock::parse(
            r#"
repository = "iii-hq/iii"
revision = "085e0fde6b424092a8b7e3ab31ac5e0cd36fa2e0"
package = "iii"
binary = "target/release/iii"
"#,
        )
        .unwrap();
        assert_eq!(lock.package, "iii");
    }

    #[test]
    fn rejects_short_or_nonhex_revisions_and_unknown_fields() {
        assert!(EngineLock::parse(
            "repository=\"r\"\nrevision=\"abc\"\npackage=\"p\"\nbinary=\"b\""
        )
        .is_err());
        assert!(EngineLock::parse(
            "repository=\"r\"\nrevision=\"zzze0fde6b424092a8b7e3ab31ac5e0cd36fa2e0\"\npackage=\"p\"\nbinary=\"b\""
        )
        .is_err());
        assert!(EngineLock::parse(
            "repository=\"r\"\nrevision=\"085e0fde6b424092a8b7e3ab31ac5e0cd36fa2e0\"\npackage=\"p\"\nbinary=\"b\"\nextra=\"x\""
        )
        .is_err());
    }
}
