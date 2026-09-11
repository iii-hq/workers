use ratatui::style::{Color, Modifier, Style};

pub fn header_accent_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn engine_url_style() -> Style {
    muted_cell_style()
}

/// Branch badge in the header. Magenta so the one identifying detail that
/// differs between instances doesn't blend into the cyan title or dim URL.
pub fn branch_style() -> Style {
    Style::default().fg(Color::Magenta)
}

pub fn footer_style() -> Style {
    muted_cell_style()
}

/// Selected row background: ~10% blue tint on a dark terminal (no fg override).
pub fn selection_row_style() -> Style {
    Style::default().bg(Color::Rgb(18, 28, 45))
}

/// compose reports `ready`, `failed` or `stopped`; the progress feed overlays
/// `queued` and `starting` while an operation is in flight.
pub fn status_style(state: &str) -> Style {
    match state {
        "ready" => Style::default().fg(Color::Green),
        "starting" | "queued" => Style::default().fg(Color::Yellow),
        "failed" => Style::default().fg(Color::Red),
        _ => muted_cell_style(),
    }
}

/// Canonical muted style. Applies DIM to the terminal's default foreground
/// instead of a fixed gray, so muted text de-emphasizes relative to the active
/// theme on both dark and light backgrounds — a fixed ANSI gray always fails one
/// polarity. Terminals without DIM fall back to normal-intensity default fg:
/// still readable, just not muted.
pub fn muted_cell_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub fn hint_style() -> Style {
    muted_cell_style()
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
