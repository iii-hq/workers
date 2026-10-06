//! Input vocabulary: keyboard HID usages (US layout) and the touch sequences
//! behind the high-level gestures. The bridge only knows raw `touch`,
//! `button` and `key` commands; everything composed lives here so it is
//! testable without a simulator.

use serde_json::{json, Value};

pub const SHIFT: u32 = 0xe1;

/// HID usage (keyboard page) of a named key or a single US-layout character,
/// and whether the character needs Shift.
pub fn key_usage(key: &str) -> Option<(u32, bool)> {
    let named = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(0x28),
        "escape" | "esc" => Some(0x29),
        "backspace" => Some(0x2a),
        "tab" => Some(0x2b),
        "space" => Some(0x2c),
        "delete" => Some(0x4c),
        "right" | "arrowright" => Some(0x4f),
        "left" | "arrowleft" => Some(0x50),
        "down" | "arrowdown" => Some(0x51),
        "up" | "arrowup" => Some(0x52),
        "home" => Some(0x4a),
        "end" => Some(0x4d),
        "pageup" => Some(0x4b),
        "pagedown" => Some(0x4e),
        "cmd" | "command" | "meta" => Some(0xe3),
        "ctrl" | "control" => Some(0xe0),
        "alt" | "option" => Some(0xe2),
        "shift" => Some(SHIFT),
        _ => None,
    };
    if let Some(usage) = named {
        return Some((usage, false));
    }
    let mut chars = key.chars();
    let (c, None) = (chars.next()?, chars.next()) else {
        return None;
    };
    char_usage(c)
}

fn char_usage(c: char) -> Option<(u32, bool)> {
    const UNSHIFTED: &str = "-=[]\\;'`,./";
    const SHIFTED: &str = "_+{}|:\"~<>?";
    const PUNCT: [u32; 11] = [
        0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38,
    ];
    const DIGIT_SHIFTED: &str = ")!@#$%^&*(";
    Some(match c {
        'a'..='z' => (0x04 + (c as u32 - 'a' as u32), false),
        'A'..='Z' => (0x04 + (c as u32 - 'A' as u32), true),
        '1'..='9' => (0x1e + (c as u32 - '1' as u32), false),
        '0' => (0x27, false),
        ' ' => (0x2c, false),
        '\n' => (0x28, false),
        '\t' => (0x2b, false),
        _ => {
            if let Some(i) = UNSHIFTED.find(c) {
                (PUNCT[i], false)
            } else if let Some(i) = SHIFTED.find(c) {
                (PUNCT[i], true)
            } else {
                let i = DIGIT_SHIFTED.find(c)?;
                (if i == 0 { 0x27 } else { 0x1e + i as u32 - 1 }, true)
            }
        }
    })
}

/// Key down/up commands typing `text`, or `None` when some character has no
/// US-keyboard key (the caller pastes it instead).
pub fn type_commands(text: &str) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    for c in text.chars() {
        let (usage, shift) = char_usage(c)?;
        if shift {
            out.push(key(SHIFT, "down"));
        }
        out.push(key(usage, "down"));
        out.push(key(usage, "up"));
        if shift {
            out.push(key(SHIFT, "up"));
        }
    }
    Some(out)
}

/// A chord (`["cmd", "v"]`): every key down in order, then up in reverse.
pub fn chord_commands(keys: &[String]) -> Result<Vec<Value>, String> {
    let mut usages = Vec::new();
    for k in keys {
        let (usage, shift) = key_usage(k).ok_or_else(|| format!("unknown key '{k}'"))?;
        if shift && !usages.contains(&SHIFT) {
            usages.push(SHIFT);
        }
        usages.push(usage);
    }
    if usages.is_empty() {
        return Err("keys is empty".into());
    }
    let mut out: Vec<Value> = usages.iter().map(|u| key(*u, "down")).collect();
    out.extend(usages.iter().rev().map(|u| key(*u, "up")));
    Ok(out)
}

fn key(usage: u32, phase: &str) -> Value {
    json!({ "op": "key", "usage": usage, "phase": phase })
}

/// One step of a composed gesture: a bridge command, then a pause.
pub type Step = (Value, u64);

fn touch(phase: &str, x: f64, y: f64, second: Option<(f64, f64)>) -> Value {
    let mut v = json!({ "op": "touch", "phase": phase, "x": x, "y": y });
    if let Some((x2, y2)) = second {
        v["x2"] = json!(x2);
        v["y2"] = json!(y2);
    }
    v
}

/// ~60 Hz moves: SimulatorKit drops drags closer than 16 ms apart.
const MOVE_MS: u64 = 17;

/// Down, ~60 Hz moves, up: finger positions as a function of progress `t`
/// in 0..=1, with an optional second finger (pinch).
fn drag(
    duration_ms: u64,
    first: impl Fn(f64) -> (f64, f64),
    second: impl Fn(f64) -> Option<(f64, f64)>,
) -> Vec<Step> {
    let steps = (duration_ms / MOVE_MS).clamp(2, 600);
    let (x0, y0) = first(0.0);
    let mut out = vec![(touch("down", x0, y0, second(0.0)), MOVE_MS)];
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let (x, y) = first(t);
        out.push((touch("move", x, y, second(t)), MOVE_MS));
    }
    let (x1, y1) = first(1.0);
    out.push((touch("up", x1, y1, second(1.0)), 0));
    out
}

pub fn tap(x: f64, y: f64, hold_ms: u64) -> Vec<Step> {
    vec![
        (touch("down", x, y, None), hold_ms.max(40)),
        (touch("up", x, y, None), 0),
    ]
}

pub fn swipe(from: (f64, f64), to: (f64, f64), duration_ms: u64) -> Vec<Step> {
    drag(
        duration_ms,
        |t| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t),
        |_| None,
    )
}

/// Two fingers on a diagonal through `center`, `from`→`to` pixels apart:
/// `to > from` zooms in, `to < from` zooms out.
pub fn pinch(center: (f64, f64), from: f64, to: f64, duration_ms: u64) -> Vec<Step> {
    let half = move |t: f64| (from + (to - from) * t) / 2.0 / std::f64::consts::SQRT_2;
    drag(
        duration_ms,
        move |t| (center.0 - half(t), center.1 - half(t)),
        move |t| Some((center.0 + half(t), center.1 + half(t))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_symbols_map_to_us_usages() {
        assert_eq!(key_usage("a"), Some((0x04, false)));
        assert_eq!(key_usage("Z"), Some((0x1d, true)));
        assert_eq!(key_usage("0"), Some((0x27, false)));
        assert_eq!(key_usage("!"), Some((0x1e, true)));
        assert_eq!(key_usage(")"), Some((0x27, true)));
        assert_eq!(key_usage("?"), Some((0x38, true)));
        assert_eq!(key_usage("Enter"), Some((0x28, false)));
        assert_eq!(key_usage("é"), None);
    }

    #[test]
    fn typing_wraps_shifted_characters() {
        let cmds = type_commands("aB").unwrap();
        let usages: Vec<(u64, &str)> = cmds
            .iter()
            .map(|c| (c["usage"].as_u64().unwrap(), c["phase"].as_str().unwrap()))
            .collect();
        assert_eq!(
            usages,
            vec![
                (0x04, "down"),
                (0x04, "up"),
                (0xe1, "down"),
                (0x05, "down"),
                (0x05, "up"),
                (0xe1, "up")
            ]
        );
        assert!(type_commands("olá").is_none());
    }

    #[test]
    fn chords_release_in_reverse() {
        let cmds = chord_commands(&["cmd".into(), "v".into()]).unwrap();
        let phases: Vec<(u64, &str)> = cmds
            .iter()
            .map(|c| (c["usage"].as_u64().unwrap(), c["phase"].as_str().unwrap()))
            .collect();
        assert_eq!(
            phases,
            vec![(0xe3, "down"), (0x19, "down"), (0x19, "up"), (0xe3, "up")]
        );
        assert!(chord_commands(&["hyper".into()]).is_err());
    }

    #[test]
    fn swipe_starts_and_ends_on_the_endpoints() {
        let steps = swipe((100.0, 800.0), (100.0, 200.0), 300);
        assert_eq!(steps.first().unwrap().0["phase"], "down");
        assert_eq!(steps.first().unwrap().0["y"], 800.0);
        let last = &steps.last().unwrap().0;
        assert_eq!(
            (last["phase"].as_str(), last["y"].as_f64()),
            (Some("up"), Some(200.0))
        );
    }

    #[test]
    fn pinch_moves_two_fingers_apart_around_the_center() {
        let steps = pinch((500.0, 1000.0), 100.0, 400.0, 200);
        let spread = |v: &Value| v["x2"].as_f64().unwrap() - v["x"].as_f64().unwrap();
        let first = &steps.first().unwrap().0;
        let last = &steps.last().unwrap().0;
        assert!(spread(last) > spread(first) * 3.9);
        let mid = (first["x"].as_f64().unwrap() + first["x2"].as_f64().unwrap()) / 2.0;
        assert!((mid - 500.0).abs() < 1e-9);
    }
}
