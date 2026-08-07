//! Stack management UI: the Ctrl+u picker and (later) creating, deleting,
//! and defaulting stacks. Split out of `tui/mod.rs`, which owns the dashboard
//! loop and was already long before stacks arrived.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use super::theme::hint_style;
use super::{draw_dialog, styled_if, UiCtx, UiMode};

/// Move the picker selection, skipping stacks with no startable roots.
/// Returns the index unchanged when there is no selectable stack in that
/// direction.
pub(super) fn move_stack_selection(
    stacks: &[(String, Vec<String>)],
    current: usize,
    down: bool,
) -> usize {
    let mut i = current;
    loop {
        if down {
            if i + 1 >= stacks.len() {
                return current;
            }
            i += 1;
        } else {
            if i == 0 {
                return current;
            }
            i -= 1;
        }
        if !stacks[i].1.is_empty() {
            return i;
        }
    }
}

/// Stack-picker keys. Enter on a startable stack just returns the chosen
/// name — `mode` is left alone here. The caller only commits (current_stack,
/// Busy, spawns the start) once `stack_members` actually resolves, so a
/// lookup error can't strand the Busy dialog up with nothing in flight.
/// Esc/q cancels back to the dashboard.
pub(super) fn handle_stack_picker_key(
    key: KeyEvent,
    mode: &mut UiMode,
    stacks: &[(String, Vec<String>)],
) -> Option<String> {
    let UiMode::StackPicker { selected } = mode else {
        return None;
    };
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            *selected = move_stack_selection(stacks, *selected, false);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            *selected = move_stack_selection(stacks, *selected, true);
        }
        KeyCode::Enter => {
            if let Some((name, roots)) = stacks.get(*selected) {
                if !roots.is_empty() {
                    return Some(name.clone());
                }
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => *mode = UiMode::Dashboard,
        _ => {}
    }
    None
}

/// Stack picker: one row per configured stack (`*` marks the default, the
/// current stack is bold, empty ones are dimmed and unselectable). Enter
/// switches the session's current stack AND starts it.
pub(super) fn draw_stack_picker_overlay(f: &mut Frame, area: Rect, selected: usize, ctx: &UiCtx) {
    let color = ctx.color_enabled;
    let mut lines = vec![Line::from("")];
    for (i, (name, roots)) in ctx.stacks.iter().enumerate() {
        let cursor = if i == selected { "▸" } else { " " };
        let marker = if name == ctx.default_stack { "*" } else { " " };
        let n = roots.len();
        let s = if n == 1 { "" } else { "s" };
        let suffix = if roots.is_empty() {
            "  (nothing startable)".to_string()
        } else {
            format!("  {n} root{s}")
        };
        let style = if roots.is_empty() {
            styled_if(color, hint_style())
        } else if i == selected {
            styled_if(color, Style::default().fg(Color::Cyan)).add_modifier(Modifier::BOLD)
        } else if name == ctx.current_stack {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("  {cursor} {marker} {name:<18}{suffix}"),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "   * default: {}   current: {}",
            ctx.default_stack, ctx.current_stack
        ),
        styled_if(color, hint_style()),
    )));
    let pinned = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "   Enter ",
                styled_if(color, Style::default().fg(Color::Cyan)),
            ),
            Span::raw("switch + start      "),
            Span::styled("Esc ", styled_if(color, Style::default().fg(Color::Cyan))),
            Span::raw("cancel"),
        ]),
        Line::from(""),
    ];
    draw_dialog(
        f,
        area,
        " start stack ".to_string(),
        lines,
        pinned,
        58,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stacks() -> Vec<(String, Vec<String>)> {
        vec![
            ("harness".to_string(), vec!["harness".to_string()]),
            ("ghost".to_string(), Vec::new()),
            ("console".to_string(), vec!["console".to_string()]),
        ]
    }

    #[test]
    fn stack_picker_selection_skips_empty_stacks_and_clamps() {
        let stacks = stacks();
        assert_eq!(move_stack_selection(&stacks, 0, true), 2); // skips ghost
        assert_eq!(move_stack_selection(&stacks, 2, false), 0); // skips ghost
        assert_eq!(move_stack_selection(&stacks, 2, true), 2); // clamp end
        assert_eq!(move_stack_selection(&stacks, 0, false), 0); // clamp start
    }

    #[test]
    fn stack_picker_enter_switches_and_esc_cancels() {
        let stacks = stacks();

        // Enter on a startable stack returns its name but leaves `mode`
        // untouched — the caller only sets Busy once `stack_members`
        // resolves, so a failed lookup can't strand a Busy dialog with
        // nothing in flight.
        let mut mode = UiMode::StackPicker { selected: 2 };
        let chosen = handle_stack_picker_key(KeyEvent::from(KeyCode::Enter), &mut mode, &stacks);
        assert_eq!(chosen.as_deref(), Some("console"));
        assert!(matches!(mode, UiMode::StackPicker { selected: 2 }));

        // Enter on an unstartable (empty) stack does nothing.
        let mut mode = UiMode::StackPicker { selected: 1 };
        let chosen = handle_stack_picker_key(KeyEvent::from(KeyCode::Enter), &mut mode, &stacks);
        assert!(chosen.is_none());
        assert!(matches!(mode, UiMode::StackPicker { selected: 1 }));

        let mut mode = UiMode::StackPicker { selected: 0 };
        let chosen = handle_stack_picker_key(KeyEvent::from(KeyCode::Esc), &mut mode, &stacks);
        assert!(chosen.is_none());
        assert!(matches!(mode, UiMode::Dashboard));
    }
}
