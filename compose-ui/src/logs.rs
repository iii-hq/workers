use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub const DEFAULT_LINES: usize = 200;
pub const MAX_LINES: usize = 500;
pub const MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct LogsInput {
    /// Container name as declared in the compose file.
    pub container: String,
    /// Lines from the end; defaults to 200 and is capped at 500.
    #[schemars(range(min = 1, max = 500))]
    pub lines: Option<i64>,
    /// Compose file on the daemon host; defaults to the daemon project.
    pub file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct LogTail {
    pub container: String,
    pub path: PathBuf,
    pub lines: Vec<String>,
    pub size: u64,
    pub truncated: bool,
    pub missing: bool,
}

pub fn valid_container_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '.' | '-')
        })
}

pub fn log_path(state_dir: &Path, container: &str) -> Result<PathBuf, String> {
    if !valid_container_name(container) {
        return Err(format!(
            "INVALID_CONTAINER: {} is not a compose container name",
            serde_json::to_string(container).unwrap_or_else(|_| format!("\"{container}\""))
        ));
    }
    Ok(state_dir.join("logs").join(format!("{container}.log")))
}

pub fn clamp_lines(requested: Option<i64>) -> usize {
    requested
        .unwrap_or(DEFAULT_LINES as i64)
        .clamp(1, MAX_LINES as i64) as usize
}

fn last_lines(text: &str, count: usize, dropped_head: bool) -> (Vec<String>, usize) {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if dropped_head && !lines.is_empty() {
        lines.remove(0);
    }
    let total = lines.len();
    let start = total.saturating_sub(count);
    (
        lines[start..]
            .iter()
            .map(|line| (*line).to_string())
            .collect(),
        total,
    )
}

pub async fn read_log_tail(
    state_dir: &Path,
    container: &str,
    requested: Option<i64>,
) -> Result<LogTail, String> {
    let path = log_path(state_dir, container)?;
    let wanted = clamp_lines(requested);
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogTail {
                container: container.to_string(),
                path,
                lines: Vec::new(),
                size: 0,
                truncated: false,
                missing: true,
            });
        }
        Err(error) => return Err(error.to_string()),
    };

    let size = file
        .metadata()
        .await
        .map_err(|error| error.to_string())?
        .len();
    let length = usize::try_from(size.min(MAX_BYTES as u64)).expect("bounded by MAX_BYTES");
    if length > 0 {
        file.seek(std::io::SeekFrom::Start(size - length as u64))
            .await
            .map_err(|error| error.to_string())?;
    }
    let mut buffer = vec![0; length];
    file.read_exact(&mut buffer)
        .await
        .map_err(|error| error.to_string())?;
    let clipped = size > length as u64;
    let text = String::from_utf8_lossy(&buffer);
    let (lines, total) = last_lines(&text, wanted, clipped);

    Ok(LogTail {
        container: container.to_string(),
        path,
        lines,
        size,
        truncated: clipped || total > wanted,
        missing: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_container_names_without_allowing_paths() {
        for name in [
            "console",
            "provider-openai",
            "db.primary",
            "worker_2",
            "3proxy",
        ] {
            assert!(valid_container_name(name), "{name}");
        }
        for name in ["", "../console", "/tmp/x", "Upper", "a/b", "a b"] {
            assert!(!valid_container_name(name), "{name}");
        }
    }

    #[test]
    fn clamps_the_requested_line_count() {
        assert_eq!(clamp_lines(None), DEFAULT_LINES);
        assert_eq!(clamp_lines(Some(0)), 1);
        assert_eq!(clamp_lines(Some(-5)), 1);
        assert_eq!(clamp_lines(Some(12)), 12);
        assert_eq!(clamp_lines(Some(10_000)), MAX_LINES);
    }

    #[tokio::test]
    async fn tails_lines_and_reports_truncation() {
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(temp.path().join("logs"))
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("logs/api.log"), "one\ntwo\nthree\nfour\n")
            .await
            .unwrap();
        let tail = read_log_tail(temp.path(), "api", Some(2)).await.unwrap();
        assert_eq!(tail.lines, ["three", "four"]);
        assert!(tail.truncated);
        assert!(!tail.missing);
    }

    #[tokio::test]
    async fn missing_logs_are_an_empty_success() {
        let temp = tempfile::tempdir().unwrap();
        let tail = read_log_tail(temp.path(), "api", None).await.unwrap();
        assert!(tail.missing);
        assert!(tail.lines.is_empty());
        assert_eq!(tail.size, 0);
    }

    #[tokio::test]
    async fn reads_only_the_bounded_tail() {
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(temp.path().join("logs"))
            .await
            .unwrap();
        let content = (0..40_000)
            .map(|line| format!("line-{line}\n"))
            .collect::<String>();
        tokio::fs::write(temp.path().join("logs/api.log"), content)
            .await
            .unwrap();
        let tail = read_log_tail(temp.path(), "api", Some(5)).await.unwrap();
        assert!(tail.truncated);
        assert_eq!(tail.lines.len(), 5);
        assert_eq!(tail.lines.last().map(String::as_str), Some("line-39999"));
    }
}
