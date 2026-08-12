//! Stack management UI: the Ctrl+u picker, creating a stack from marked
//! workers, and deleting or defaulting stacks from that same picker. Split
//! out of `tui/mod.rs`, which owns the dashboard loop and was already long
//! before stacks arrived.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::status::WorkerView;

use super::theme::hint_style;
use super::{draw_dialog, styled_if, UiCtx, UiMode};

/// Move the picker selection by one row, clamped to the list's bounds.
/// Every stack is reachable, including ones with no startable roots: `x`
/// (delete) must be able to reach them (deleting an unstartable stack is
/// exactly what you want the picker for), and Enter/`*` separately refuse to
/// act on an empty one rather than making it unreachable.
pub(super) fn move_stack_selection(
    stacks: &[(String, Vec<String>)],
    current: usize,
    down: bool,
) -> usize {
    if down {
        (current + 1).min(stacks.len().saturating_sub(1))
    } else {
        current.saturating_sub(1)
    }
}

/// What the picker's keys ask the dashboard loop to do.
pub(super) enum PickerAction {
    /// Switch the current stack to this one and start it (Enter).
    Start(String),
    /// Remove this stack from the config file (x) — caller confirms first.
    Delete(String),
    /// Point `default_stack:` at this one (*).
    MakeDefault(String),
}

/// Stack-picker keys. Enter on a startable stack just returns
/// `PickerAction::Start` — `mode` is left alone here. The caller only commits
/// (current_stack, Busy, spawns the start) once `stack_members` actually
/// resolves, so a lookup error can't strand the Busy dialog up with nothing
/// in flight. `x` returns Delete for the highlighted stack regardless of
/// whether it has startable roots — an empty stack is exactly the one you'd
/// want to delete. `*` shares Enter's non-empty-roots guard instead: an empty
/// stack can never become the default. Esc/q cancels back to the dashboard.
pub(super) fn handle_stack_picker_key(
    key: KeyEvent,
    mode: &mut UiMode,
    stacks: &[(String, Vec<String>)],
) -> Option<PickerAction> {
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
                    return Some(PickerAction::Start(name.clone()));
                }
            }
        }
        KeyCode::Char('x') => {
            if let Some((name, _)) = stacks.get(*selected) {
                return Some(PickerAction::Delete(name.clone()));
            }
        }
        KeyCode::Char('*') => {
            // Unlike `x`, this is guarded: writing `default_stack:` at an
            // empty-roots stack would pass `write_verified`'s validation
            // (it checks stack-key types, not default membership/emptiness)
            // and then brick the next `Config::load` — the exact failure
            // class the previous task's Critical fix was about.
            if let Some((name, roots)) = stacks.get(*selected) {
                if !roots.is_empty() {
                    return Some(PickerAction::MakeDefault(name.clone()));
                }
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => *mode = UiMode::Dashboard,
        _ => {}
    }
    None
}

/// Why `name` can't be deleted right now, or `None` if the picker should go
/// ahead and open the confirm dialog. The only reason today: deleting the
/// default stack would leave `default_stack:` pointing at nothing, and
/// `Config::load` would refuse to start on the next launch — this guard is
/// the one thing standing between the picker and that outcome, so it's
/// pulled out here to be unit-tested on its own rather than trusted to a
/// `rustc`-checked string literal buried in `run()`'s dispatch.
pub(super) fn refuse_delete_reason(name: &str, default_stack: &str) -> Option<String> {
    if name == default_stack {
        Some("set another default first (*) before deleting the default stack".to_string())
    } else {
        None
    }
}

/// Stack picker: one row per configured stack (`*` marks the default, the
/// current stack is bold, empty ones are dimmed but still reachable — `x`
/// can delete them). Enter switches the session's current stack AND starts
/// it.
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
            Span::raw("switch + start  "),
            Span::styled("x ", styled_if(color, Style::default().fg(Color::Cyan))),
            Span::raw("delete  "),
            Span::styled("* ", styled_if(color, Style::default().fg(Color::Cyan))),
            Span::raw("default  "),
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

/// The marked workers in dashboard order — the order the user sees, so the
/// saved file reads the way the list did.
pub(super) fn roots_from_marks(views: &[WorkerView], marked: &HashSet<String>) -> Vec<String> {
    views
        .iter()
        .filter(|v| marked.contains(&v.name))
        .map(|v| v.name.clone())
        .collect()
}

/// Name-prompt keys. Enter on a non-empty name returns `(name, roots)`; the
/// caller decides whether that name overwrites an existing stack. Esc cancels.
/// Characters that would need YAML quoting are ignored as typed.
pub(super) fn handle_name_key(key: KeyEvent, mode: &mut UiMode) -> Option<(String, Vec<String>)> {
    let UiMode::NameStack { name, roots, .. } = mode else {
        return None;
    };
    match key.code {
        KeyCode::Char(c) if crate::config_write::valid_stack_name(&c.to_string()) => name.push(c),
        KeyCode::Backspace => {
            name.pop();
        }
        KeyCode::Enter if !name.is_empty() => return Some((name.clone(), roots.clone())),
        KeyCode::Esc => *mode = UiMode::Dashboard,
        _ => {}
    }
    None
}

/// Read `path`'s current contents, or an empty string if it doesn't exist yet
/// (genuinely "nothing to load yet", e.g. a fresh repo's first save). Any
/// other read failure (bad permissions, a directory in the way, invalid
/// UTF-8) must NOT be treated the same way — that would build a fresh
/// document out of nothing and have `write_verified` rename it over whatever
/// was actually there.
fn read_or_empty(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

/// Write `name`'s stack into the config file, creating the file if needed.
pub(super) fn save_stack(path: &Path, name: &str, roots: &[String]) -> Result<()> {
    let text = read_or_empty(path)?;
    let next = crate::config_write::upsert_stack(&text, name, roots)?;
    crate::config_write::write_verified(path, &next)
        .with_context(|| format!("save stack {name} to {}", path.display()))
}

/// Remove `name`'s entry from the config file.
pub(super) fn delete_stack(path: &Path, name: &str) -> Result<()> {
    let text = read_or_empty(path)?;
    let next = crate::config_write::remove_stack(&text, name)?;
    crate::config_write::write_verified(path, &next)
        .with_context(|| format!("delete stack {name} from {}", path.display()))
}

/// Point `default_stack:` at `name`.
pub(super) fn set_default(path: &Path, name: &str) -> Result<()> {
    let text = read_or_empty(path)?;
    let next = crate::config_write::set_default_stack(&text, name)?;
    crate::config_write::write_verified(path, &next)
        .with_context(|| format!("set default stack {name} in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::WorkerGroup;

    fn stacks() -> Vec<(String, Vec<String>)> {
        vec![
            ("harness".to_string(), vec!["harness".to_string()]),
            ("ghost".to_string(), Vec::new()),
            ("console".to_string(), vec!["console".to_string()]),
        ]
    }

    /// Shared worker fixture — mirrors `status.rs`'s test fixture of the same
    /// name/shape.
    fn view(name: &str) -> WorkerView {
        WorkerView {
            name: name.to_string(),
            group: WorkerGroup::Other,
            spawnable: true,
            display_status: "stopped".to_string(),
            process_status: "stopped".to_string(),
            engine_status: "—".to_string(),
            local_pid: None,
            uptime: "—".to_string(),
            exit_code: None,
            ui_watch: None,
        }
    }

    #[test]
    fn stack_picker_selection_reaches_empty_stacks_and_clamps() {
        let stacks = stacks();
        // `ghost` (index 1) has no roots but must still be reachable — it's
        // the only way `x` can ever delete it.
        assert_eq!(move_stack_selection(&stacks, 0, true), 1);
        assert_eq!(move_stack_selection(&stacks, 2, false), 1);
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
        assert!(matches!(chosen, Some(PickerAction::Start(ref n)) if n == "console"));
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

    #[test]
    fn picker_keys_return_start_delete_and_default_actions() {
        let stacks = vec![
            ("harness".to_string(), vec!["harness".to_string()]),
            ("ghost".to_string(), Vec::new()),
            ("console".to_string(), vec!["console".to_string()]),
        ];

        let mut mode = UiMode::StackPicker { selected: 2 };
        assert!(matches!(
            handle_stack_picker_key(KeyEvent::from(KeyCode::Enter), &mut mode, &stacks),
            Some(PickerAction::Start(ref n)) if n == "console"
        ));
        assert!(matches!(
            handle_stack_picker_key(KeyEvent::from(KeyCode::Char('x')), &mut mode, &stacks),
            Some(PickerAction::Delete(ref n)) if n == "console"
        ));
        assert!(matches!(
            handle_stack_picker_key(KeyEvent::from(KeyCode::Char('*')), &mut mode, &stacks),
            Some(PickerAction::MakeDefault(ref n)) if n == "console"
        ));

        // An unstartable stack can still be deleted, but not started or made
        // default (the latter would write a `default_stack:` that bricks the
        // next `Config::load`).
        let mut mode = UiMode::StackPicker { selected: 1 };
        assert!(
            handle_stack_picker_key(KeyEvent::from(KeyCode::Enter), &mut mode, &stacks).is_none()
        );
        assert!(
            handle_stack_picker_key(KeyEvent::from(KeyCode::Char('*')), &mut mode, &stacks)
                .is_none()
        );
        assert!(matches!(
            handle_stack_picker_key(KeyEvent::from(KeyCode::Char('x')), &mut mode, &stacks),
            Some(PickerAction::Delete(ref n)) if n == "ghost"
        ));

        // Esc still cancels.
        let mut mode = UiMode::StackPicker { selected: 0 };
        assert!(
            handle_stack_picker_key(KeyEvent::from(KeyCode::Esc), &mut mode, &stacks).is_none()
        );
        assert!(matches!(mode, UiMode::Dashboard));
    }

    #[test]
    fn refuse_delete_reason_blocks_only_the_default() {
        let reason = refuse_delete_reason("console", "console").unwrap();
        assert!(
            reason.contains("before deleting the default stack"),
            "{reason:?}"
        );
        assert!(refuse_delete_reason("console", "harness").is_none());
    }

    #[test]
    fn delete_and_set_default_edit_the_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        std::fs::write(
            &path,
            "stacks:\n  console:\n    - console\n  tiny:\n    - session-manager\n",
        )
        .unwrap();

        set_default(&path, "console").unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("default_stack: console"));

        delete_stack(&path, "tiny").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("  console:"));
        assert!(!text.contains("tiny"));

        // A stack that isn't in the file (e.g. the built-in harness) says so.
        let err = delete_stack(&path, "harness").unwrap_err();
        assert!(
            err.to_string().contains("not defined in this file"),
            "{err:#}"
        );
    }

    /// A failed write (`remove_stack` refusing a duplicated `stacks:` key)
    /// must leave the file exactly as it was — mirrors
    /// `save_stack_leaves_the_file_untouched_on_error`.
    #[test]
    fn delete_stack_leaves_the_file_untouched_on_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        let original = "stacks:\n  a:\n    - x\nstacks:\n  b:\n    - y\n";
        std::fs::write(&path, original).unwrap();

        let err = delete_stack(&path, "a").unwrap_err();
        assert!(err.to_string().contains("more than once"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    /// A read failure that isn't "file doesn't exist yet" must propagate —
    /// mirrors `save_stack_propagates_read_errors_other_than_not_found`, and
    /// is the regression guard for `delete_stack`'s own NotFound-only default.
    #[test]
    fn delete_stack_propagates_read_errors_other_than_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        std::fs::create_dir(&path).unwrap();

        let err = delete_stack(&path, "console").unwrap_err();
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "{err:#}"
        );
        assert!(path.is_dir());
    }

    /// A failed write (`set_default_stack` refusing a duplicated
    /// `default_stack:` key) must leave the file exactly as it was — mirrors
    /// `save_stack_leaves_the_file_untouched_on_error`.
    #[test]
    fn set_default_leaves_the_file_untouched_on_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        let original = "default_stack: a\ndefault_stack: b\n";
        std::fs::write(&path, original).unwrap();

        let err = set_default(&path, "console").unwrap_err();
        assert!(err.to_string().contains("more than once"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    /// A read failure that isn't "file doesn't exist yet" must propagate —
    /// mirrors `save_stack_propagates_read_errors_other_than_not_found`, and
    /// is the regression guard for `set_default`'s own NotFound-only default.
    #[test]
    fn set_default_propagates_read_errors_other_than_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        std::fs::create_dir(&path).unwrap();

        let err = set_default(&path, "console").unwrap_err();
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "{err:#}"
        );
        assert!(path.is_dir());
    }

    #[test]
    fn roots_from_marks_follows_dashboard_order() {
        let mut views = vec![view("zeta"), view("alpha"), view("mid")];
        // zeta is a stack member: it sorts into the Stack group, ahead of the
        // alphabetically-earlier Other group, so dashboard order is
        // zeta, alpha, mid — deliberately NOT alphabetical. A `marks.sort()`
        // reimplementation would produce alpha, zeta and fail this.
        let members: HashSet<String> = ["zeta".to_string()].into_iter().collect();
        crate::status::assign_view_groups(&mut views, &members);
        let marked: HashSet<String> = ["zeta".to_string(), "alpha".to_string()]
            .into_iter()
            .collect();
        assert_eq!(
            roots_from_marks(&views, &marked),
            vec!["zeta".to_string(), "alpha".to_string()]
        );
    }

    #[test]
    fn name_key_edits_accepts_and_cancels() {
        let roots = vec!["console".to_string()];

        // Valid characters append; invalid ones are ignored.
        let mut mode = UiMode::NameStack {
            name: String::new(),
            roots: roots.clone(),
            expanded: 3,
        };
        for c in ['c', 'o', ' ', 'n', ':', '-', '1'] {
            assert!(handle_name_key(KeyEvent::from(KeyCode::Char(c)), &mut mode).is_none());
        }
        let UiMode::NameStack { name, .. } = &mode else {
            panic!("still naming")
        };
        assert_eq!(name, "con-1");

        // Backspace edits.
        handle_name_key(KeyEvent::from(KeyCode::Backspace), &mut mode);
        let UiMode::NameStack { name, .. } = &mode else {
            panic!("still naming")
        };
        assert_eq!(name, "con-");

        // Enter returns the name and its roots.
        let out = handle_name_key(KeyEvent::from(KeyCode::Enter), &mut mode).unwrap();
        assert_eq!(out, ("con-".to_string(), roots.clone()));

        // Enter on an empty name does nothing.
        let mut mode = UiMode::NameStack {
            name: String::new(),
            roots: roots.clone(),
            expanded: 3,
        };
        assert!(handle_name_key(KeyEvent::from(KeyCode::Enter), &mut mode).is_none());
        assert!(matches!(mode, UiMode::NameStack { .. }));

        // Esc cancels.
        handle_name_key(KeyEvent::from(KeyCode::Esc), &mut mode);
        assert!(matches!(mode, UiMode::Dashboard));
    }

    #[test]
    fn save_stack_creates_then_updates_the_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");

        save_stack(&path, "console", &["console".to_string()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "stacks:\n  console:\n    - console\n"
        );

        save_stack(
            &path,
            "console",
            &["console".to_string(), "state".to_string()],
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "stacks:\n  console:\n    - console\n    - state\n"
        );
    }

    /// A failed write (here: `upsert_stack` refusing a duplicated `stacks:`
    /// key) must leave the file exactly as it was — this is the property the
    /// error-banner path in `save_and_adopt_stack` depends on.
    #[test]
    fn save_stack_leaves_the_file_untouched_on_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        let original = "stacks:\n  a:\n    - x\nstacks:\n  b:\n    - y\n";
        std::fs::write(&path, original).unwrap();

        let err = save_stack(&path, "console", &["console".to_string()]).unwrap_err();
        assert!(err.to_string().contains("more than once"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    /// A read failure that isn't "file doesn't exist yet" (here: a directory
    /// sitting where the config file should be) must propagate, not be
    /// treated as an empty file — that would silently overwrite whatever was
    /// actually unreadable there with a fresh single-stack document.
    #[test]
    fn save_stack_propagates_read_errors_other_than_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        std::fs::create_dir(&path).unwrap();

        let err = save_stack(&path, "console", &["console".to_string()]).unwrap_err();
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "{err:#}"
        );
        // Untouched — still a directory, not clobbered by a written file.
        assert!(path.is_dir());
    }
}
