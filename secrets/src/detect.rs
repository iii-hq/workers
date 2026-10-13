//! Finding credentials that already exist on this machine, so the onboarding
//! wizard can offer "import" instead of "paste":
//!
//! - `process_env`: this worker's own environment;
//! - `dotenv`: `<III_COMPOSE_DIR>/.env`;
//! - `login_shell`: `$SHELL -ilc` printing a marker then `env -0`.
//!
//! Values found here only ever leave this module masked (`hint`) or as a
//! fingerprint comparison; `secrets::import` re-reads them itself. A source
//! that fails (no file, a broken shell rc, a timeout) contributes nothing; it
//! never fails the call.
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::names::is_valid_name;
use crate::secret::SecretString;

pub const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(4);
/// More than any real `env -0` dump; the read stops here.
#[cfg_attr(not(unix), allow(dead_code))]
const LOGIN_SHELL_MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// After the shell exits, how long to keep draining a pipe that a daemon it
/// started may still hold open.
#[cfg_attr(not(unix), allow(dead_code))]
const LOGIN_SHELL_DRAIN: Duration = Duration::from_millis(250);
pub const PROCESS_ENV_LOCATION: &str = "secrets worker environment";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    ProcessEnv,
    Dotenv,
    LoginShell,
}

/// One non-empty value for one name in one source.
#[derive(Debug, Clone)]
pub struct Found {
    pub name: String,
    pub kind: SourceKind,
    pub location: String,
    pub value: SecretString,
}

/// Where each source looks. Built from the process environment at boot;
/// tests build their own.
#[derive(Debug, Clone)]
pub struct Detector {
    pub process_env: bool,
    pub dotenv: Option<PathBuf>,
    pub shell: Option<PathBuf>,
    pub shell_timeout: Duration,
}

impl Detector {
    pub fn from_env() -> Self {
        Self {
            process_env: true,
            // The worker reads the configured env file per call (`Ctx`).
            dotenv: Some(iii_worker_paths::project_path(".env")),
            shell: std::env::var_os("SHELL")
                .filter(|shell| !shell.is_empty())
                .map(PathBuf::from),
            shell_timeout: LOGIN_SHELL_TIMEOUT,
        }
    }

    /// Every source's values for `names` (already validated), in source order.
    pub async fn scan(&self, names: &[String]) -> Vec<Found> {
        let mut found = Vec::new();
        for kind in [
            SourceKind::ProcessEnv,
            SourceKind::Dotenv,
            SourceKind::LoginShell,
        ] {
            found.extend(self.read(kind, names).await);
        }
        found
    }

    /// One source's values for `names`.
    pub async fn read(&self, kind: SourceKind, names: &[String]) -> Vec<Found> {
        if names.is_empty() {
            return Vec::new();
        }
        match kind {
            SourceKind::ProcessEnv if self.process_env => read_process_env(names),
            SourceKind::ProcessEnv => Vec::new(),
            SourceKind::Dotenv => match &self.dotenv {
                Some(path) => read_dotenv(path, names).await,
                None => Vec::new(),
            },
            SourceKind::LoginShell => match &self.shell {
                Some(shell) => read_login_shell(shell, names, self.shell_timeout).await,
                None => Vec::new(),
            },
        }
    }
}

fn found(name: &str, kind: SourceKind, location: &str, value: SecretString) -> Option<Found> {
    (!value.is_blank()).then(|| Found {
        name: name.to_owned(),
        kind,
        location: location.to_owned(),
        value,
    })
}

fn read_process_env(names: &[String]) -> Vec<Found> {
    names
        .iter()
        .filter_map(|name| {
            let value = std::env::var(name).ok()?;
            found(
                name,
                SourceKind::ProcessEnv,
                PROCESS_ENV_LOCATION,
                value.into(),
            )
        })
        .collect()
}

async fn read_dotenv(path: &std::path::Path, names: &[String]) -> Vec<Found> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => Zeroizing::new(text),
        Err(_) => return Vec::new(),
    };
    let mut entries = parse_dotenv(&text);
    let location = path.display().to_string();
    names
        .iter()
        .filter_map(|name| {
            let value = entries.remove(name)?;
            found(name, SourceKind::Dotenv, &location, value)
        })
        .collect()
}

/// `KEY=VALUE` lines as a dotenv loader reads them; a later definition of
/// the same key wins. See [`scan_dotenv`] for the grammar.
pub fn parse_dotenv(text: &str) -> BTreeMap<String, SecretString> {
    scan_dotenv(text)
        .into_iter()
        .map(|entry| (entry.key, entry.value))
        .collect()
}

/// One definition in a dotenv file.
#[derive(Debug)]
pub struct DotenvEntry {
    pub key: String,
    pub value: SecretString,
    /// From the start of its first line through its last line's newline.
    pub span: std::ops::Range<usize>,
}

/// Every definition, in file order:
///
/// - blank lines and `#` comment lines are skipped, as is a leading `export `;
/// - `'single'` quotes are literal; `"double"` quotes understand `\n`, `\r`,
///   `\t`, `\\` and `\"`; either may span lines; text after the closing
///   quote is ignored;
/// - an unquoted value ends at ` #` / `\t#` (inline comment) and is trimmed;
/// - lines without `=` or with an invalid key are skipped, as is an
///   unterminated quote.
pub fn scan_dotenv(text: &str) -> Vec<DotenvEntry> {
    let mut entries = Vec::new();
    let len = text.len();
    let mut pos = 0;
    while pos < len {
        let line_start = pos;
        let line_end = text[pos..].find('\n').map_or(len, |i| pos + i);
        let line = &text[pos..line_end];
        pos = line_end + 1;
        let body = line.trim_start();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        let body = body
            .strip_prefix("export")
            .filter(|rest| rest.starts_with([' ', '\t']))
            .map_or(body, str::trim_start);
        let Some(eq) = body.find('=') else {
            continue;
        };
        let key = body[..eq].trim();
        if !is_valid_name(key) {
            continue;
        }
        let raw = &body[eq + 1..];
        let value = raw.trim_start_matches([' ', '\t']);
        // `value` is a suffix of `line`, so its start in `text` is fixed.
        let value_start = line_end - value.len();
        let parsed = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => {
                let Some((parsed, close)) = quoted(text, value_start + 1, quote) else {
                    continue;
                };
                // Skip whatever follows the closing quote on its line.
                pos = text[close..].find('\n').map_or(len, |i| close + i + 1);
                parsed
            }
            _ => {
                let end = raw
                    .as_bytes()
                    .windows(2)
                    .position(|pair| matches!(pair[0], b' ' | b'\t') && pair[1] == b'#')
                    .unwrap_or(raw.len());
                raw[..end].trim_matches([' ', '\t', '\r']).into()
            }
        };
        entries.push(DotenvEntry {
            key: key.to_owned(),
            value: parsed,
            span: line_start..pos.min(len),
        });
    }
    entries
}

/// The quoted value starting at `start` (just past the opening quote) and
/// the index just past the closing quote.
fn quoted(text: &str, start: usize, quote: char) -> Option<(SecretString, usize)> {
    let mut out = Zeroizing::new(String::new());
    let mut chars = text[start..].char_indices();
    while let Some((offset, c)) = chars.next() {
        match c {
            c if c == quote => {
                return Some((
                    SecretString::new(std::mem::take(&mut out)),
                    start + offset + 1,
                ))
            }
            '\\' if quote == '"' => match chars.next()? {
                (_, 'n') => out.push('\n'),
                (_, 'r') => out.push('\r'),
                (_, 't') => out.push('\t'),
                (_, '\\') => out.push('\\'),
                (_, '"') => out.push('"'),
                (_, other) => {
                    out.push('\\');
                    out.push(other);
                }
            },
            c => out.push(c),
        }
    }
    None
}

/// The `env -0` entries printed after `marker`, kept only for `wanted`
/// names. `None` without the marker (the shell never ran the command). A
/// trailing segment without its NUL terminator is dropped: it is output from
/// a logout hook, not an environment entry.
pub fn parse_env_block(
    stdout: &[u8],
    marker: &str,
    wanted: &HashSet<&str>,
) -> Option<BTreeMap<String, SecretString>> {
    let marker = marker.as_bytes();
    let at = stdout
        .windows(marker.len())
        .position(|window| window == marker)?;
    let block = &stdout[at + marker.len()..];
    let mut entries = BTreeMap::new();
    let mut segments = block.split(|&b| b == 0).peekable();
    while let Some(entry) = segments.next() {
        if segments.peek().is_none() {
            break;
        }
        let Some(eq) = entry.iter().position(|&b| b == b'=') else {
            continue;
        };
        let Ok(key) = std::str::from_utf8(&entry[..eq]) else {
            continue;
        };
        if !wanted.contains(key) {
            continue;
        }
        if let Ok(value) = std::str::from_utf8(&entry[eq + 1..]) {
            entries.insert(key.to_owned(), value.into());
        }
    }
    Some(entries)
}

async fn read_login_shell(
    shell: &std::path::Path,
    names: &[String],
    timeout: Duration,
) -> Vec<Found> {
    let marker = format!("__III_SECRETS_ENV_{}__", uuid::Uuid::new_v4().simple());
    let Some(stdout) = run_login_shell(shell, &marker, timeout).await else {
        return Vec::new();
    };
    let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
    let Some(mut entries) = parse_env_block(&stdout, &marker, &wanted) else {
        tracing::debug!(shell = %shell.display(), "login shell printed no environment");
        return Vec::new();
    };
    let location = shell.display().to_string();
    names
        .iter()
        .filter_map(|name| {
            let value = entries.remove(name)?;
            found(name, SourceKind::LoginShell, &location, value)
        })
        .collect()
}

/// Run `shell -ilc 'printf marker; env -0'` with stdin closed, stderr
/// discarded and stdout captured, in its own session (no controlling
/// terminal for an interactive rc to grab). `None` on any failure, including
/// the deadline, after which the whole process group is killed.
#[cfg(unix)]
async fn run_login_shell(
    shell: &std::path::Path,
    marker: &str,
    timeout: Duration,
) -> Option<Zeroizing<Vec<u8>>> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;

    let mut command = tokio::process::Command::new(shell);
    command
        .arg("-ilc")
        .arg(format!("printf '%s' '{marker}'; env -0"))
        .env("III_RESOLVING_ENVIRONMENT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // SAFETY: setsid is async-signal-safe and touches no parent state.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::debug!(shell = %shell.display(), error = %error.kind(), "login shell did not start");
            return None;
        }
    };
    let pid = child.id();
    let mut stdout = child.stdout.take()?;
    let mut output = Zeroizing::new(Vec::with_capacity(64 * 1024));
    let mut chunk = Zeroizing::new([0u8; 8192]);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut stop_at = deadline;
    let mut exited = false;
    let mut overflow = false;
    let mut eof = false;
    loop {
        tokio::select! {
            read = stdout.read(chunk.as_mut()) => match read {
                Ok(0) | Err(_) => {
                    eof = true;
                    break;
                }
                Ok(n) => {
                    if output.len() + n > LOGIN_SHELL_MAX_OUTPUT {
                        overflow = true;
                        break;
                    }
                    output.extend_from_slice(&chunk[..n]);
                }
            },
            _ = child.wait(), if !exited => {
                exited = true;
                stop_at = deadline.min(tokio::time::Instant::now() + LOGIN_SHELL_DRAIN);
            }
            _ = tokio::time::sleep_until(stop_at) => break,
        }
    }
    if !exited && eof {
        // Output is complete; the exit status may simply not be reaped yet.
        exited = tokio::time::timeout_at(deadline, child.wait())
            .await
            .is_ok();
    }
    if !exited {
        if let Some(pid) = pid.and_then(|pid| i32::try_from(pid).ok()) {
            // SAFETY: plain syscall on the group this child leads (setsid).
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
        tracing::warn!(shell = %shell.display(), timeout_ms = timeout.as_millis() as u64, "login shell timed out; skipping it as a source");
        return None;
    }
    if overflow {
        tracing::warn!(shell = %shell.display(), "login shell output too large; skipping it as a source");
        return None;
    }
    Some(output)
}

#[cfg(not(unix))]
async fn run_login_shell(
    _shell: &std::path::Path,
    _marker: &str,
    _timeout: Duration,
) -> Option<Zeroizing<Vec<u8>>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(entries: &BTreeMap<String, SecretString>, key: &str) -> Option<String> {
        entries.get(key).map(|v| v.expose().to_owned())
    }

    #[test]
    fn dotenv_parses_the_common_forms() {
        let text = "\
# a comment
   # an indented comment

ANTHROPIC_API_KEY=sk-ant-plain
export OPENAI_API_KEY=sk-openai # inline comment
exported=not-an-export-prefix
SINGLE='lit #not comment \\n'
DOUBLE=\"line1\\nline2 \\\"q\\\"\" # trailing
SPACED =  padded value  \t
HASH=#kept
EMPTY=
EMPTY_COMMENT= # nothing
MULTI=\"first
second\"
AFTER=yes
BROKEN=\"never closed
9BAD=x
no equals here
CRLF=windows\r
DUP=first
DUP=second
";
        let entries = parse_dotenv(text);
        assert_eq!(
            value(&entries, "ANTHROPIC_API_KEY").as_deref(),
            Some("sk-ant-plain")
        );
        assert_eq!(
            value(&entries, "OPENAI_API_KEY").as_deref(),
            Some("sk-openai")
        );
        assert_eq!(
            value(&entries, "exported").as_deref(),
            Some("not-an-export-prefix")
        );
        assert_eq!(
            value(&entries, "SINGLE").as_deref(),
            Some("lit #not comment \\n")
        );
        assert_eq!(
            value(&entries, "DOUBLE").as_deref(),
            Some("line1\nline2 \"q\"")
        );
        assert_eq!(value(&entries, "SPACED").as_deref(), Some("padded value"));
        assert_eq!(value(&entries, "HASH").as_deref(), Some("#kept"));
        assert_eq!(value(&entries, "EMPTY").as_deref(), Some(""));
        assert_eq!(value(&entries, "EMPTY_COMMENT").as_deref(), Some(""));
        assert_eq!(value(&entries, "MULTI").as_deref(), Some("first\nsecond"));
        assert_eq!(value(&entries, "AFTER").as_deref(), Some("yes"));
        assert_eq!(value(&entries, "CRLF").as_deref(), Some("windows"));
        assert_eq!(value(&entries, "DUP").as_deref(), Some("second"));
        assert!(!entries.contains_key("9BAD"));
        assert!(!entries.contains_key("BROKEN"));
    }

    #[test]
    fn dotenv_export_requires_whitespace() {
        let entries = parse_dotenv("export\tTABBED=1\nexportX=2\n");
        assert_eq!(value(&entries, "TABBED").as_deref(), Some("1"));
        assert_eq!(value(&entries, "exportX").as_deref(), Some("2"));
    }

    #[test]
    fn env_block_parses_after_the_marker_only() {
        let marker = "__M__";
        let mut stdout = b"motd: welcome\nFAKE=from-rc\0".to_vec();
        stdout.extend_from_slice(marker.as_bytes());
        stdout.extend_from_slice(b"PATH=/bin\0OPENAI_API_KEY=sk-from-shell\0MULTI=a\nb\0EQ=x=y\0OTHER=ignored\0bye from zlogout");
        let wanted: HashSet<&str> = ["OPENAI_API_KEY", "MULTI", "EQ", "FAKE", "bye from zlogout"]
            .into_iter()
            .collect();
        let entries = parse_env_block(&stdout, marker, &wanted).unwrap();
        assert_eq!(
            value(&entries, "OPENAI_API_KEY").as_deref(),
            Some("sk-from-shell")
        );
        assert_eq!(value(&entries, "MULTI").as_deref(), Some("a\nb"));
        assert_eq!(value(&entries, "EQ").as_deref(), Some("x=y"));
        // Before the marker: rc output, not the environment.
        assert!(!entries.contains_key("FAKE"));
        // Not requested: never retained.
        assert!(!entries.contains_key("OTHER") && !entries.contains_key("PATH"));
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn env_block_without_marker_is_none() {
        let wanted = HashSet::from(["A"]);
        assert!(parse_env_block(b"A=1\0", "__M__", &wanted).is_none());
        let empty = parse_env_block(b"__M__", "__M__", &wanted).unwrap();
        assert!(empty.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn login_shell_reads_the_environment() {
        let root = tempfile::tempdir().unwrap();
        let shell = root.path().join("fake-shell");
        // A stand-in login shell: rc noise on stdout, then the -c script.
        std::fs::write(
            &shell,
            "#!/bin/sh\necho 'rc says hi'\nexport FROM_LOGIN_SHELL=sk-login-shell-value\nshift\nexec /bin/sh -c \"$1\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let names = vec!["FROM_LOGIN_SHELL".to_owned(), "MISSING_ONE".to_owned()];
        let found = read_login_shell(&shell, &names, Duration::from_secs(4)).await;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SourceKind::LoginShell);
        assert_eq!(found[0].value.expose(), "sk-login-shell-value");
        assert_eq!(found[0].location, shell.display().to_string());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hanging_or_broken_shell_contributes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let hanging = root.path().join("hang");
        std::fs::write(&hanging, "#!/bin/sh\nsleep 30\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hanging, std::fs::Permissions::from_mode(0o755)).unwrap();
        let names = vec!["X".to_owned()];
        let started = std::time::Instant::now();
        assert!(
            read_login_shell(&hanging, &names, Duration::from_millis(300))
                .await
                .is_empty()
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        let missing = root.path().join("does-not-exist");
        assert!(read_login_shell(&missing, &names, Duration::from_secs(1))
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn detector_reads_dotenv_and_skips_blank_values() {
        let root = tempfile::tempdir().unwrap();
        let dotenv = root.path().join(".env");
        std::fs::write(&dotenv, "A_KEY=from-dotenv\nBLANK=   \n").unwrap();
        let detector = Detector {
            process_env: false,
            dotenv: Some(dotenv.clone()),
            shell: None,
            shell_timeout: Duration::from_secs(1),
        };
        let found = detector
            .scan(&["A_KEY".to_owned(), "BLANK".to_owned()])
            .await;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, SourceKind::Dotenv);
        assert_eq!(found[0].location, dotenv.display().to_string());
        assert!(!format!("{found:?}").contains("from-dotenv"));
    }
}
