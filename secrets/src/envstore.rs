//! The `env` store: a secret kept as an environment variable rather than in
//! the vault. Its value is the variable in the project's `.env`
//! (`<III_COMPOSE_DIR>/.env`) or, for a deploy that has none, in this
//! worker's own environment, read each time it is resolved. `secrets::set`
//! with `store: "env"` writes it to `.env`, leaving every other line as it
//! was.
//!
//! The vault records only who may read each variable (`EnvGrant`), never its
//! value.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::detect::{scan_dotenv, PROCESS_ENV_LOCATION};
use crate::error::SecretsError;
use crate::fsutil;
use crate::secret::SecretString;

/// A variable's current value and where it was read from.
#[derive(Debug, Clone)]
pub struct EnvValue {
    pub value: SecretString,
    /// The `.env` path, or [`PROCESS_ENV_LOCATION`].
    pub location: String,
}

/// Where the env store reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvSource {
    /// The project's `.env`; `None` leaves only the process environment.
    pub dotenv: Option<PathBuf>,
    /// Fall back to this worker's environment (off in tests).
    pub process_env: bool,
}

impl EnvSource {
    /// Current values of `names`: `.env` first, the file being what the
    /// console edits; then this worker's environment. Blank values count as
    /// unset.
    pub fn read(&self, names: &[String]) -> BTreeMap<String, EnvValue> {
        let mut out = BTreeMap::new();
        if names.is_empty() {
            return out;
        }
        if let Some(path) = &self.dotenv {
            if let Ok(text) = std::fs::read_to_string(path) {
                let text = Zeroizing::new(text);
                let location = path.display().to_string();
                for entry in scan_dotenv(&text) {
                    if names.contains(&entry.key) && !entry.value.is_blank() {
                        // A later definition wins, as a dotenv loader reads it.
                        out.insert(
                            entry.key,
                            EnvValue {
                                value: entry.value,
                                location: location.clone(),
                            },
                        );
                    } else if names.contains(&entry.key) {
                        out.remove(&entry.key);
                    }
                }
            }
        }
        if self.process_env {
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                if let Some(value) = std::env::var(name).ok().filter(|v| !v.trim().is_empty()) {
                    out.insert(
                        name.clone(),
                        EnvValue {
                            value: value.into(),
                            location: PROCESS_ENV_LOCATION.to_owned(),
                        },
                    );
                }
            }
        }
        out
    }

    pub fn read_one(&self, name: &str) -> Option<EnvValue> {
        self.read(&[name.to_owned()]).remove(name)
    }

    /// Set `name` in `.env` (created owner-only when missing). Returns the
    /// file's path.
    pub fn write(&self, name: &str, value: &SecretString) -> Result<String, SecretsError> {
        let path = self.dotenv.as_deref().ok_or_else(|| {
            SecretsError::invalid_request(
                "this secrets worker has no project .env to write to (III_COMPOSE_DIR is not set)",
            )
        })?;
        let line = Zeroizing::new(assignment(name, value.expose())?);
        // Write through a symlink to the file it points at.
        let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let current = match std::fs::read_to_string(&target) {
            Ok(text) => Zeroizing::new(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Zeroizing::new(String::new())
            }
            Err(error) => {
                return Err(SecretsError::vault(format!(
                    "{} could not be read: {}",
                    target.display(),
                    error.kind()
                )))
            }
        };
        let next = Zeroizing::new(upsert(&current, name, &line));
        fsutil::write_private_atomic(&target, next.as_bytes()).map_err(|error| {
            SecretsError::vault(format!(
                "{} could not be written: {}",
                target.display(),
                error.kind()
            ))
        })?;
        Ok(path.display().to_string())
    }

    pub fn dotenv_path(&self) -> Option<&Path> {
        self.dotenv.as_deref()
    }
}

/// `NAME=value`, quoted only when it has to be, in a form that this worker's
/// parser and Compose's `env_file` reader both read back unchanged. A value
/// neither can hold (a line break, or both kinds of quote) is refused.
pub fn assignment(name: &str, value: &str) -> Result<String, SecretsError> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(SecretsError::invalid_request(
            "a value kept in .env must be a single line; store it in the vault instead",
        ));
    }
    let bare = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./+:@=,%~^".contains(c));
    if bare {
        return Ok(format!("{name}={value}"));
    }
    if !value.contains('\'') {
        return Ok(format!("{name}='{value}'"));
    }
    if !value.contains(['"', '\\']) {
        return Ok(format!("{name}=\"{value}\""));
    }
    Err(SecretsError::invalid_request(
        "this value cannot be written to .env unambiguously; store it in the vault instead",
    ))
}

/// `text` with `name` defined once, by `line`: the first definition is
/// replaced in place and later ones are dropped; with none, an empty
/// commented placeholder (`# NAME=`) is replaced; otherwise `line` is
/// appended. Every other byte is kept.
pub fn upsert(text: &str, name: &str, line: &str) -> String {
    let spans: Vec<_> = scan_dotenv(text)
        .into_iter()
        .filter(|entry| entry.key == name)
        .map(|entry| entry.span)
        .collect();
    let mut out = String::with_capacity(text.len() + line.len() + 1);
    if !spans.is_empty() {
        let mut at = 0;
        for (index, span) in spans.iter().enumerate() {
            out.push_str(&text[at..span.start]);
            if index == 0 {
                out.push_str(line);
                out.push('\n');
            }
            at = span.end;
        }
        out.push_str(&text[at..]);
        return out;
    }
    if let Some(span) = placeholder(text, name) {
        out.push_str(&text[..span.start]);
        out.push_str(line);
        out.push('\n');
        out.push_str(&text[span.end..]);
        return out;
    }
    out.push_str(text);
    if !text.is_empty() && !text.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    out
}

/// The first comment line that is only `# NAME=` (optionally `export`ed):
/// what an env template leaves for a key to be filled in.
fn placeholder(text: &str, name: &str) -> Option<std::ops::Range<usize>> {
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        let body = line.trim();
        if let Some(comment) = body.strip_prefix('#') {
            let comment = comment.trim_start();
            let comment = comment
                .strip_prefix("export")
                .filter(|rest| rest.starts_with([' ', '\t']))
                .map_or(comment, str::trim_start);
            if comment
                .strip_prefix(name)
                .and_then(|rest| rest.trim_start().strip_prefix('='))
                .is_some_and(|rest| rest.trim().is_empty())
            {
                return Some(start..end);
            }
        }
        start = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::parse_dotenv;

    fn value_of(text: &str, name: &str) -> Option<String> {
        parse_dotenv(text)
            .get(name)
            .map(|value| value.expose().to_owned())
    }

    #[test]
    fn assignments_round_trip_through_both_readers() {
        for value in [
            "sk-ant-api03-abc_DEF.123",
            "has space",
            "with#hash",
            "it's",
            "dollar$sign",
            "a=b",
        ] {
            let line = assignment("K", value).unwrap();
            assert_eq!(value_of(&line, "K").as_deref(), Some(value), "{line}");
            // Compose's env_file reader: trim, split at the first '=', strip
            // one layer of matching quotes.
            let raw = line.split_once('=').unwrap().1.trim();
            let compose = raw
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .or_else(|| {
                    raw.strip_prefix('\'')
                        .and_then(|rest| rest.strip_suffix('\''))
                })
                .unwrap_or(raw);
            assert_eq!(compose, value, "{line}");
        }
        for value in ["two\nlines", "both ' and \"", "back\\slash'"] {
            assert!(assignment("K", value).is_err(), "{value:?}");
        }
    }

    #[test]
    fn upsert_replaces_in_place_and_keeps_everything_else() {
        let text = "# keys\nA=1\nexport B=\"old\" # note\nC=3\nB=dup\n";
        let next = upsert(text, "B", "B=new");
        assert_eq!(next, "# keys\nA=1\nB=new\nC=3\n");
        assert_eq!(value_of(&next, "B").as_deref(), Some("new"));
        // A multi-line quoted definition is replaced whole.
        let text = "A=\"one\ntwo\"\nC=3";
        assert_eq!(upsert(text, "A", "A=x"), "A=x\nC=3");
    }

    #[test]
    fn upsert_fills_a_placeholder_or_appends() {
        let text = "# Paste one key\n# MOONSHOT_API_KEY=\n# XAI_API_KEY=xai-example\nA=1";
        let next = upsert(text, "MOONSHOT_API_KEY", "MOONSHOT_API_KEY=sk-1");
        assert_eq!(
            next,
            "# Paste one key\nMOONSHOT_API_KEY=sk-1\n# XAI_API_KEY=xai-example\nA=1"
        );
        // A commented example with a value is documentation, not a slot.
        let next = upsert(text, "XAI_API_KEY", "XAI_API_KEY=xai-2");
        assert!(next.ends_with("A=1\nXAI_API_KEY=xai-2\n"), "{next}");
        assert_eq!(upsert("", "A", "A=1"), "A=1\n");
    }

    #[test]
    fn read_prefers_dotenv_then_the_process_environment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "A=from-file\nB=\nA=later-wins\n").unwrap();
        let source = EnvSource {
            dotenv: Some(path.clone()),
            process_env: true,
        };
        std::env::set_var("SECRETS_ENVSTORE_TEST_B", "from-process");
        let names = vec![
            "A".to_owned(),
            "B".to_owned(),
            "SECRETS_ENVSTORE_TEST_B".to_owned(),
        ];
        let values = source.read(&names);
        assert_eq!(values["A"].value.expose(), "later-wins");
        assert_eq!(values["A"].location, path.display().to_string());
        assert!(!values.contains_key("B"), "blank is unset");
        assert_eq!(
            values["SECRETS_ENVSTORE_TEST_B"].location,
            PROCESS_ENV_LOCATION
        );
        std::env::remove_var("SECRETS_ENVSTORE_TEST_B");
    }

    #[test]
    fn write_creates_an_owner_only_file_and_rewrites_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let source = EnvSource {
            dotenv: Some(path.clone()),
            process_env: false,
        };
        source.write("A", &"first-value".into()).unwrap();
        std::fs::write(&path, "# comment\nA=first-value\nZ=keep\n").unwrap();
        source.write("A", &"second value".into()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# comment\nA='second value'\nZ=keep\n"
        );
        #[cfg(unix)]
        assert_eq!(fsutil::mode_of(&path).unwrap(), fsutil::FILE_MODE);
        let none = EnvSource {
            dotenv: None,
            process_env: false,
        };
        assert!(none.write("A", &"v".into()).is_err());
    }
}
