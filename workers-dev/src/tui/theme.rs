use ratatui::style::{Color, Modifier, Style};

use crate::discover::WorkerGroup;

pub fn header_accent_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn engine_url_style() -> Style {
    Style::default().fg(Color::Gray)
}

pub fn footer_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Selected row background: ~10% blue tint on a dark terminal (no fg override).
pub fn selection_row_style() -> Style {
    Style::default().bg(Color::Rgb(18, 28, 45))
}

pub fn group_header_style(group: WorkerGroup) -> Style {
    match group {
        WorkerGroup::HarnessStack => Style::default().fg(Color::Cyan),
        WorkerGroup::Other => Style::default().fg(Color::DarkGray),
    }
}

pub fn status_style(display_status: &str) -> Style {
    match display_status {
        "connected" => Style::default().fg(Color::Green),
        "compiling" | "disconnected" => Style::default().fg(Color::Yellow),
        "crashed" => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::DarkGray),
    }
}

pub fn process_style(process_status: &str) -> Style {
    match process_status {
        "running" => Style::default().fg(Color::Green),
        "compiling" => Style::default().fg(Color::Yellow),
        "crashed" => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::DarkGray),
    }
}

pub fn engine_style(engine_status: &str) -> Style {
    match engine_status {
        "connected" => Style::default().fg(Color::Green),
        "—" => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::Yellow),
    }
}

pub fn muted_cell_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn non_spawnable_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn hint_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn confirm_prompt_style() -> Style {
    Style::default().fg(Color::Yellow)
}

pub fn overlay_bg_style() -> Style {
    Style::default().bg(Color::Black)
}

pub fn log_title_style() -> Style {
    Style::default().fg(Color::Cyan)
}
