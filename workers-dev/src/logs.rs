use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Normalize process output for TUI display and ring-buffer storage.
pub fn normalize_log_line(raw: &str) -> String {
    let segment = raw.rsplit('\r').next().unwrap_or(raw).trim_end();
    let stripped = strip_ansi(segment);
    stripped.trim_end().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    CargoProgress,
    CargoDone,
    Error,
    Warn,
    Info,
    Debug,
    Plain,
}

pub fn classify_log_line(line: &str) -> LogKind {
    if line.contains("process exited")
        || line.contains("could not compile")
        || line.contains("error[E")
        || line.contains("error:")
    {
        return LogKind::Error;
    }
    if line.contains("Compiling")
        || line.contains("Building")
        || line.contains("Downloading")
        || line.contains("Updating")
        || line.contains("Blocking waiting")
    {
        return LogKind::CargoProgress;
    }
    if line.contains("Finished `") && line.contains("profile") || line.contains("Running `target/")
    {
        return LogKind::CargoDone;
    }
    if contains_level_token(line, "WARN") {
        return LogKind::Warn;
    }
    if contains_level_token(line, "ERROR") {
        return LogKind::Error;
    }
    if contains_level_token(line, "INFO") {
        return LogKind::Info;
    }
    if contains_level_token(line, "DEBUG") || contains_level_token(line, "TRACE") {
        return LogKind::Debug;
    }
    LogKind::Plain
}

fn contains_level_token(line: &str, level: &str) -> bool {
    line.contains(&format!(" {level} "))
        || line.contains(&format!(" {level}:"))
        || line.contains(&format!(" {level}\t"))
}

pub fn log_line_to_ratatui(line: &str, max_width: usize, color_enabled: bool) -> Line<'static> {
    if max_width == 0 {
        return Line::from("");
    }

    if !color_enabled {
        return Line::from(truncate_chars(line, max_width));
    }

    if let Some((ts, rest)) = split_tracing_timestamp(line) {
        let kind = classify_log_line(line);
        let ts_str = truncate_chars(ts, max_width);
        let ts_width: usize = ts_str.chars().map(unicode_width).sum();
        let mut spans = vec![Span::styled(ts_str, log_timestamp_style(true))];
        // Only append the separator + message when columns remain. If the
        // timestamp alone fills max_width, a trailing space would push the line
        // one column past the pane and wrap it onto a second row.
        if ts_width < max_width {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                truncate_chars(rest, max_width - ts_width - 1),
                log_kind_style(kind, true),
            ));
        }
        return Line::from(spans);
    }

    let kind = classify_log_line(line);
    Line::from(Span::styled(
        truncate_chars(line, max_width),
        log_kind_style(kind, true),
    ))
}

fn log_kind_style(kind: LogKind, color_enabled: bool) -> Style {
    if !color_enabled {
        return Style::default();
    }
    match kind {
        LogKind::CargoProgress | LogKind::Warn => Style::default().fg(Color::Yellow),
        LogKind::CargoDone => Style::default().fg(Color::Cyan),
        LogKind::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        LogKind::Info => Style::default().fg(Color::Green),
        LogKind::Debug => Style::default().fg(Color::DarkGray),
        LogKind::Plain => Style::default(),
    }
}

fn log_timestamp_style(color_enabled: bool) -> Style {
    if color_enabled {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    }
}

fn split_tracing_timestamp(line: &str) -> Option<(&str, &str)> {
    if !line.starts_with("20") {
        return None;
    }
    let sep = line.find("  ")?;
    if !(20..=40).contains(&sep) {
        return None;
    }
    let ts = &line[..sep];
    if !ts.contains('T') {
        return None;
    }
    Some((ts, line[sep..].trim_start()))
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn truncate_chars(input: &str, max_width: usize) -> String {
    let mut width = 0usize;
    let mut out = String::new();
    for ch in input.chars() {
        let w = unicode_width(ch);
        if width + w > max_width {
            break;
        }
        width += w;
        out.push(ch);
    }
    out
}

fn unicode_width(ch: char) -> usize {
    if ch.is_ascii() {
        1
    } else {
        2
    }
}

/// Last `bytes` of a log file, normalized for the pane. Serves both the
/// container logs compose retains under `<state_dir>/logs/` and the compose
/// daemon's own redirect file, so there is one reader, not two.
///
/// Seeks rather than reads: compose lets the active segment grow to 10 MiB
/// before rotating, and this runs on every poll tick.
pub fn tail_file(path: &Path, bytes: u64) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let from_start = len <= bytes;
    if !from_start && file.seek(SeekFrom::End(-(bytes as i64))).is_err() {
        return Vec::new();
    }

    // Decoded lossily rather than read line by line: `BufRead::lines()` yields
    // an error for a line that is not valid UTF-8 and the iterator ends there,
    // so a seek landing between the bytes of a glyph would throw the whole tail
    // away — and the daemon log is mostly →, ✓, ✗ and ·.
    let mut window = Vec::new();
    if BufReader::new(file).read_to_end(&mut window).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&window);

    let mut lines: Vec<String> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        // A mid-line seek lands inside a record; the first fragment is not a line.
        if index == 0 && !from_start {
            continue;
        }
        // compose stamps each worker log with a format header and prefixes
        // every record with its stream. Our own daemon log has neither.
        if index == 0 && from_start && line.starts_with("# iii-compose-worker-log-") {
            continue;
        }
        let body = line
            .strip_prefix("stdout\t")
            .or_else(|| line.strip_prefix("stderr\t"))
            .unwrap_or(line);
        lines.push(normalize_log_line(body));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_and_carriage_return_overwrites() {
        let raw = "    Blocking waiting\r    Blocking waiting for file lock\r    Compiling harness v1.0.0";
        assert_eq!(normalize_log_line(raw), "    Compiling harness v1.0.0");
    }

    #[test]
    fn strips_color_codes() {
        let raw = "\x1b[2m2026-06-22T15:47:56 INFO\x1b[0m harness: ready";
        assert_eq!(
            normalize_log_line(raw),
            "2026-06-22T15:47:56 INFO harness: ready"
        );
    }

    #[test]
    fn classify_cargo_and_tracing() {
        assert_eq!(
            classify_log_line("   Compiling harness v1.0.0"),
            LogKind::CargoProgress
        );
        assert_eq!(
            classify_log_line(
                "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.21s"
            ),
            LogKind::CargoDone
        );
        assert_eq!(
            classify_log_line("2026-06-22T15:47:56.851170Z  INFO harness: ready"),
            LogKind::Info
        );
        assert_eq!(
            classify_log_line("2026-06-22T15:47:56.851170Z  WARN harness::config: bad"),
            LogKind::Warn
        );
        assert_eq!(
            classify_log_line("error[E0425]: cannot find value `foo`"),
            LogKind::Error
        );
    }

    #[test]
    fn log_line_to_ratatui_respects_width() {
        let line = log_line_to_ratatui("   Compiling harness v1.0.0", 10, true);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "   Compili");
    }

    #[test]
    fn log_line_to_ratatui_timestamp_fits_narrow_pane() {
        // Timestamp alone fills the pane — the trailing separator must not push
        // the rendered line one column past max_width (would wrap onto two rows).
        let max = 10;
        let line = log_line_to_ratatui(
            "2026-06-22T15:47:56.851170Z  INFO harness: ready",
            max,
            true,
        );
        let total: usize = line
            .spans
            .iter()
            .flat_map(|s| s.content.chars())
            .map(unicode_width)
            .sum();
        assert!(total <= max, "rendered width {total} exceeds {max}");
    }

    #[test]
    fn tail_file_strips_the_compose_header_and_stream_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.log");
        std::fs::write(
            &path,
            "# iii-compose-worker-log-v1 abc\nstdout\thello\nstderr\tboom\nplain\n",
        )
        .unwrap();
        assert_eq!(
            super::tail_file(&path, 64 * 1024),
            ["hello", "boom", "plain"]
        );
        assert!(super::tail_file(&dir.path().join("missing.log"), 64).is_empty());
    }

    // The daemon log is full of →, ✓, ✗ and ·, so a seek lands mid-glyph often.
    #[test]
    fn tail_file_survives_a_seek_into_the_middle_of_a_glyph() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.log");
        let body: String = (0..400)
            .map(|i| format!("✓ container-{i} ready\n"))
            .collect();
        std::fs::write(&path, &body).unwrap();
        let lines = super::tail_file(&path, 64);
        assert!(
            !lines.is_empty(),
            "a mid-glyph seek must not discard the tail"
        );
        assert_eq!(lines.last().unwrap(), "✓ container-399 ready");
    }

    #[test]
    fn tail_file_drops_the_partial_first_line_after_a_seek() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        let body: String = (0..500).map(|i| format!("line-{i}\n")).collect();
        std::fs::write(&path, &body).unwrap();
        let lines = super::tail_file(&path, 64);
        assert!(lines.len() < 500 && !lines.is_empty());
        assert_eq!(lines.last().unwrap(), "line-499");
    }
}
