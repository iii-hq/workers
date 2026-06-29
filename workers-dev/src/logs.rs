use std::io::Write;

use crossterm::style::{Color as CrosstermColor, ResetColor, SetForegroundColor};
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
        let mut spans = vec![
            Span::styled(truncate_chars(ts, max_width), log_timestamp_style(true)),
            Span::raw(" "),
        ];
        let rest_width = max_width.saturating_sub(ts.chars().count().min(max_width) + 1);
        spans.push(Span::styled(
            truncate_chars(rest, rest_width),
            log_kind_style(kind, true),
        ));
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

fn log_crossterm_color(kind: LogKind) -> Option<CrosstermColor> {
    use crossterm::style::Color as CrosstermColor;
    match kind {
        LogKind::CargoProgress | LogKind::Warn => Some(CrosstermColor::Yellow),
        LogKind::CargoDone => Some(CrosstermColor::Cyan),
        LogKind::Error => Some(CrosstermColor::Red),
        LogKind::Info => Some(CrosstermColor::Green),
        LogKind::Debug => Some(CrosstermColor::DarkGrey),
        LogKind::Plain => None,
    }
}

pub fn print_colored_line(
    line: &str,
    color_enabled: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    if !color_enabled {
        writeln!(out, "{line}")?;
        return Ok(());
    }
    if let Some(color) = log_crossterm_color(classify_log_line(line)) {
        crossterm::execute!(out, SetForegroundColor(color))?;
        write!(out, "{line}")?;
        crossterm::execute!(out, ResetColor)?;
    } else {
        write!(out, "{line}")?;
    }
    writeln!(out)?;
    Ok(())
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
}
