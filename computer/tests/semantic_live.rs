//! Real-desktop e2e of the semantic layer against the AccessKit fixture
//! (computer/tests/fixture). Opt-in and never silently skipped:
//!
//! ```sh
//! cargo build --locked --manifest-path tests/fixture/Cargo.toml
//! COMPUTER_A11Y_FIXTURE=$PWD/tests/fixture/target/debug/computer-a11y-fixture \
//!   cargo test --locked --test semantic_live -- --ignored --test-threads=1
//! ```
//!
//! macOS needs Accessibility and Screen Recording for the terminal running
//! it; Windows needs an interactive desktop (hosted windows-latest has one).
//! Linux runs it through tests/run-linux-live.sh (private Xvfb + D-Bus + AT-SPI).
//! `COMPUTER_A11Y_FIXTURE_LOG_DIR` picks where the fixture logs go (default:
//! the temp dir); CI uploads them when the job fails.
#![cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use computer::driver::{DisplayInfo, Driver, NativeHost};
use computer::semantic::{parse_selector, platform, Code, Query, Scope, SemanticSession, Verb};

const SECRET: &str = "s3cr3t-fixture";
const SECRET_B: &str = "campo-b-s3cr3t";

struct Fixture {
    child: Child,
    log: PathBuf,
}

/// `window` line: client-area origin and size (physical px) and scale factor.
#[derive(Clone, Copy)]
struct Win {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    scale: f64,
}

impl Fixture {
    /// The fixture's window is up (it logged it); its tree may not be yet.
    async fn launch() -> Self {
        let bin = std::env::var("COMPUTER_A11Y_FIXTURE")
            .expect("set COMPUTER_A11Y_FIXTURE to the built computer-a11y-fixture binary");
        static RUN: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::var_os("COMPUTER_A11Y_FIXTURE_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let log = dir.join(format!(
            "a11y-fixture-{}-{}.log",
            std::process::id(),
            RUN.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_file(&log);
        let child = Command::new(bin)
            .env("COMPUTER_A11Y_FIXTURE_LOG", &log)
            .spawn()
            .expect("spawn the fixture");
        let f = Fixture { child, log };
        let start = Instant::now();
        while f.window().is_none() {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "fixture never logged its window"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        f
    }

    /// Launched, and its real accessibility tree is readable. AccessKit
    /// serves an empty placeholder to the first query and builds the tree on
    /// its event loop, so poll instead of sleeping a guess.
    async fn start() -> Self {
        let f = Self::launch().await;
        let probe = session();
        let start = Instant::now();
        loop {
            let got = probe
                .snapshot(query(f.pid(), Some("button[name='Remover']")), false)
                .await;
            if matches!(&got, Ok(o) if o.matches == 1) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "fixture tree never appeared; last read: {}",
                match got {
                    Ok(o) => format!("{} matches\n{}", o.matches, o.text),
                    Err(e) => format!("{:?}: {}", e.code, e.message),
                }
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        f
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// Every accessibility action request the fixture received, Focus included.
    fn requests(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|l| l.starts_with("action "))
            .collect()
    }

    /// The requests that are an act's effect: all but Focus. UI Automation
    /// calls `IRawElementProviderFragment::SetFocus` on the element before
    /// Invoke/Toggle/SetValue/Select, and AccessKit logs that as its own
    /// Focus request; AX and AT-SPI send none.
    fn effects(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|l| !is_focus(l))
            .collect()
    }

    fn window(&self) -> Option<Win> {
        let line = self
            .lines()
            .into_iter()
            .find(|l| l.starts_with("window "))?;
        let v: Vec<&str> = line[7..].split(' ').collect();
        Some(Win {
            x: v[0].parse().unwrap(),
            y: v[1].parse().unwrap(),
            w: v[2].parse().unwrap(),
            h: v[3].parse().unwrap(),
            scale: v[4].parse().unwrap(),
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn session() -> SemanticSession {
    SemanticSession::new(platform::factory())
}

fn query(pid: u32, selector: Option<&str>) -> Query {
    Query {
        scope: Scope {
            app_pid: Some(pid),
            ..Default::default()
        },
        selector: selector.map(|s| parse_selector(s).unwrap()),
        max_nodes: 256,
        max_depth: 12,
    }
}

fn refs(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("[ref="))
        .map(|l| l.split(']').next().unwrap().to_string())
        .collect()
}

fn first_ref(text: &str) -> String {
    refs(text)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no ref in:\n{text}"))
}

fn ref_on(text: &str, needle: &str) -> String {
    let line = text
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle} in:\n{text}"));
    first_ref(line)
}

/// Before a negative check: time for a stray action to show up in the log.
fn settle() {
    std::thread::sleep(Duration::from_millis(400));
}

/// Poll `cond` for up to 5 s: the fixture applies an action on its own
/// event loop, after the accessibility call already returned.
async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "never happened: {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn find(s: &SemanticSession, pid: u32, selector: &str) -> computer::semantic::SnapshotOut {
    s.snapshot(query(pid, Some(selector)), false).await.unwrap()
}

/// Retry the read until it reports `n` matches, for up to 5 s. An `Err` (the
/// fixture rebuilding after an act can make an element vanish mid-read) counts
/// as "not yet"; the last outcome goes in the timeout message.
async fn wait_matches(s: &SemanticSession, pid: u32, selector: &str, n: usize) {
    let start = Instant::now();
    loop {
        let last = match s.snapshot(query(pid, Some(selector)), false).await {
            Ok(out) if out.matches == n => return,
            Ok(out) => format!("{} matches, want {n}\n{}", out.matches, out.text),
            Err(e) => format!("read failed: {e:?}"),
        };
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{selector}: {last}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `action <target> <Action> <data>`: the request's target element.
fn target(request: &str) -> &str {
    request.split(' ').nth(1).unwrap_or_default()
}

fn is_focus(request: &str) -> bool {
    request.split(' ').nth(2) == Some("Focus")
}

/// Exactly `n` effect requests logged: wait for the n-th, then make sure no
/// extra one follows. A Focus is allowed only directly before an effect on the
/// same element, so a doubled Focus or a Focus without an act still fails.
async fn exactly_actions(f: &Fixture, n: usize) {
    wait_for(&format!("{n} fixture actions"), || f.effects().len() >= n).await;
    settle();
    let all = f.requests();
    assert_eq!(f.effects().len(), n, "{all:?}");
    for (i, r) in all.iter().enumerate().filter(|(_, r)| is_focus(r)) {
        assert!(
            all.get(i + 1)
                .is_some_and(|next| !is_focus(next) && target(next) == target(r)),
            "Focus without its act: {all:?}"
        );
    }
}

#[tokio::test]
#[ignore]
async fn invoke_dispatches_exactly_once() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "button[name='Salvar']").await;
    assert_eq!(out.matches, 1, "{}", out.text);
    s.act(first_ref(&out.text), Verb::Invoke, None)
        .await
        .unwrap();
    exactly_actions(&f, 1).await;
    assert!(
        f.effects()[0].starts_with("action Salvar Click"),
        "{:?}",
        f.requests()
    );
    wait_matches(&s, f.pid(), "[name^='Status: salvo 1']", 1).await;
}

#[tokio::test]
#[ignore]
async fn set_value_round_trips_unicode() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "text_field[name='Nome']").await;
    s.act(
        first_ref(&out.text),
        Verb::SetValue,
        Some("ação, João, €".into()),
    )
    .await
    .unwrap();
    exactly_actions(&f, 1).await;
    wait_matches(
        &s,
        f.pid(),
        "text_field[name='Nome'][value='ação, João, €']",
        1,
    )
    .await;
}

#[tokio::test]
#[ignore]
async fn toggle_and_select_change_observed_state() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "check_box[name='Ativo']").await;
    s.act(first_ref(&out.text), Verb::Toggle, None)
        .await
        .unwrap();
    // Counts, not names: how UIA/AX toggle and select map onto AccessKit
    // actions is the platform's business; one effect request per act is ours.
    exactly_actions(&f, 1).await;
    wait_matches(&s, f.pid(), "check_box[name='Ativo'][checked='on']", 1).await;
    let out = find(&s, f.pid(), "[name='Dois']").await;
    s.act(first_ref(&out.text), Verb::Select, None)
        .await
        .unwrap();
    exactly_actions(&f, 2).await;
    wait_matches(&s, f.pid(), "[name='Dois'][selected='true']", 1).await;
}

#[tokio::test]
#[ignore]
async fn duplicates_are_listed_never_picked() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "button[name='OK']").await;
    assert_eq!(out.matches, 2, "{}", out.text);
    settle();
    assert!(f.requests().is_empty(), "{:?}", f.requests());
}

#[tokio::test]
#[ignore]
async fn a_disabled_button_is_refused_without_effect() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "button[name='Desabilitado']").await;
    let err = s
        .act(first_ref(&out.text), Verb::Invoke, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, Code::NotSupported);
    settle();
    assert!(f.requests().is_empty(), "{:?}", f.requests());
    assert_eq!(
        find(&s, f.pid(), "[name^='Status: salvo 0']").await.matches,
        1
    );
}

#[tokio::test]
#[ignore]
async fn select_refuses_what_has_no_selection_and_flags_a_selected_item() {
    let f = Fixture::start().await;
    let s = session();
    // A button has no selection state on any platform: refused, no request.
    let out = find(&s, f.pid(), "button[name='Salvar']").await;
    let err = s
        .act(first_ref(&out.text), Verb::Select, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, Code::NotSupported, "{}", err.message);
    settle();
    assert!(f.requests().is_empty(), "{:?}", f.requests());
    // Um starts selected: dispatched, said so, and AccessKit does nothing.
    let out = find(&s, f.pid(), "[name='Um'][selected='true']").await;
    assert_eq!(out.matches, 1, "{}", out.text);
    let got = s
        .act(first_ref(&out.text), Verb::Select, None)
        .await
        .unwrap();
    assert!(got.contains("already selected"), "{got}");
    settle();
    assert!(f.effects().is_empty(), "{:?}", f.requests());
}

#[tokio::test]
#[ignore]
async fn a_read_only_field_is_refused_without_effect() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "text_field[name='Fixo']").await;
    assert_eq!(out.matches, 1, "{}", out.text);
    let got = s
        .act(first_ref(&out.text), Verb::SetValue, Some("novo".into()))
        .await;
    // ponytail: on macOS an AccessKit node answers AXValue settable whatever
    // it accepts (the setter is overridden), so the act is dispatched
    // unverified and AppKit drops it (vendor/xa11y-macos PATCHES.md, M2).
    if cfg!(not(target_os = "macos")) {
        assert_eq!(got.unwrap_err().code, Code::NotSupported);
    }
    settle();
    assert!(f.requests().is_empty(), "{:?}", f.requests());
    wait_matches(&s, f.pid(), "text_field[name='Fixo'][value='fixo']", 1).await;
}

#[tokio::test]
#[ignore]
async fn a_removed_element_is_a_stale_ref() {
    let f = Fixture::start().await;
    let s = session();
    let tree = s.snapshot(query(f.pid(), None), true).await.unwrap();
    let salvar = ref_on(&tree.text, "\"Salvar\"");
    let remover = ref_on(&tree.text, "\"Remover\"");
    s.act(remover, Verb::Invoke, None).await.unwrap();
    // Wait on the log, not on a find: a find with matches would replace the
    // snapshot and make the next act stale for the wrong reason.
    exactly_actions(&f, 1).await;
    let before = f.requests();
    let err = s.act(salvar, Verb::Invoke, None).await.unwrap_err();
    assert_eq!(err.code, Code::StaleRef, "{}", err.message);
    settle();
    assert_eq!(f.requests(), before);
}

#[tokio::test]
#[ignore]
async fn a_replaced_snapshot_is_a_stale_ref() {
    let f = Fixture::start().await;
    let s = session();
    let old = first_ref(&find(&s, f.pid(), "button[name='Salvar']").await.text);
    find(&s, f.pid(), "button[name='Remover']").await;
    assert_eq!(
        s.act(old, Verb::Invoke, None).await.unwrap_err().code,
        Code::StaleRef
    );
    settle();
    assert!(f.requests().is_empty(), "{:?}", f.requests());
}

#[tokio::test]
#[ignore]
async fn the_password_value_never_leaves() {
    let f = Fixture::start().await;
    let s = session();
    let tree = s.snapshot(query(f.pid(), None), true).await.unwrap();
    assert!(tree.text.contains("Senha"), "{}", tree.text);
    assert!(!tree.text.contains(SECRET));
    // Positive control: the field is there, so 0 below means the value hid it.
    assert_eq!(find(&s, f.pid(), "[name='Senha']").await.matches, 1);
    assert_eq!(
        find(&s, f.pid(), "[name='Senha'][value^='s']")
            .await
            .matches,
        0
    );
    assert_eq!(
        parse_selector("[uia_value^='s']").unwrap_err().code,
        Code::InvalidSelector
    );
    assert_eq!(
        parse_selector("[AXValue^='s']").unwrap_err().code,
        Code::InvalidSelector
    );
    // A neutral name: only the password role can hide it (AT-SPI, AX). UIA
    // redacts by name only, so Windows shows it (a documented limit).
    if cfg!(not(target_os = "windows")) {
        assert!(tree.text.contains("Campo B"), "{}", tree.text);
        assert!(!tree.text.contains(SECRET_B), "{}", tree.text);
        assert_eq!(find(&s, f.pid(), "[name='Campo B']").await.matches, 1);
        assert_eq!(
            find(&s, f.pid(), "[name='Campo B'][value^='c']")
                .await
                .matches,
            0
        );
    }
    assert_eq!(
        parse_selector("[atspi_value^='s']").unwrap_err().code,
        Code::InvalidSelector
    );
    settle();
    assert!(f.requests().is_empty(), "{:?}", f.requests());
}

/// Click the centre of the fixture window through the coordinate driver,
/// mapping screen coordinates into the session's (downscaled) image space.
/// The fixture logs physical pixels; `list_displays` sizes are what `xcap`
/// reports — physical on Windows (per-monitor-v2) and Linux, logical points
/// on macOS. The pointer is parked off the window first, so the click has to
/// travel: one that lands where the pointer was cannot pass by luck.
async fn click_center(host: &NativeHost, display: &DisplayInfo, win: Win) {
    host.screenshot().await.unwrap();
    let image = host.screen_size().await.unwrap();
    let s = if cfg!(target_os = "macos") {
        win.scale
    } else {
        1.0
    };
    let to_image = |x: i64, y: i64| {
        let (tx, ty) = ((x as f64 / s) as i64, (y as f64 / s) as i64);
        (
            tx * image.width as i64 / display.width as i64,
            ty * image.height as i64 / display.height as i64,
        )
    };
    let (px, py) = to_image(win.x / 2, win.y / 2);
    host.move_cursor(px, py).await.unwrap();
    let (ix, iy) = to_image(win.x + win.w / 2, win.y + win.h / 2);
    host.left_click(ix, iy).await.unwrap();
}

fn mouse_clicks(f: &Fixture) -> Vec<(i64, i64)> {
    f.lines()
        .into_iter()
        .filter_map(|l| l.strip_prefix("mouse ").map(String::from))
        .map(|l| {
            let v: Vec<i64> = l.split(' ').map(|n| n.parse().unwrap()).collect();
            (v[0], v[1])
        })
        .collect()
}

/// The primary display, or the first when none is primary (Xvfb marks none).
fn main_display() -> DisplayInfo {
    let all = computer::driver::native::list_displays().unwrap();
    all.iter()
        .find(|d| d.primary)
        .or(all.first())
        .cloned()
        .expect("a display")
}

/// Smoke test: a coordinate click on the fixture lands, and still lands after
/// a semantic call in the same process.
#[tokio::test]
#[ignore]
async fn a_coordinate_click_lands_before_and_after_semantic_use() {
    // No readiness probe: the first click must precede any semantic call.
    let f = Fixture::launch().await;
    // The window is logged at creation; give it a moment to be shown.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let window = f.window().unwrap();
    let (ww, wh) = (window.w, window.h);
    let display = main_display();
    // Capture the display the click is mapped through, not the one under the
    // pointer: they differ on a multi-monitor desktop.
    let host = NativeHost::new(4096, 80, Some(display.index));
    click_center(&host, &display, window).await;
    wait_for("the first click", || mouse_clicks(&f).len() == 1).await;
    let s = session();
    s.snapshot(query(f.pid(), None), true).await.unwrap();
    click_center(&host, &display, window).await;
    wait_for("the second click", || mouse_clicks(&f).len() == 2).await;
    settle();
    let clicks = mouse_clicks(&f);
    assert_eq!(clicks.len(), 2, "{:?}", f.lines());
    for (cx, cy) in clicks {
        assert!(
            (cx - ww / 2).abs() <= 2 && (cy - wh / 2).abs() <= 2,
            "click at ({cx}, {cy}) in {ww}x{wh}"
        );
    }
}

/// The private session of run-linux-live.sh turns accessibility on, so the
/// probe must say so with no warning.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore]
async fn the_live_bus_reports_accessibility_on() {
    assert_eq!(platform::a11y_support().await, (true, None));
}

/// What the fixture's window received as text, `escape_debug`-ed per key.
#[cfg(target_os = "linux")]
fn typed(f: &Fixture) -> String {
    f.lines()
        .iter()
        .filter_map(|l| l.strip_prefix("key "))
        .collect()
}

/// X11 typing, including characters missing from Xvfb's US map (bound to a
/// spare keycode for the call): the fixture's window receives them, and
/// Return arrives as "\r".
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore]
async fn type_reaches_the_window_including_unmapped_characters() {
    let f = Fixture::launch().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let window = f.window().unwrap();
    let host = NativeHost::new(4096, 80, None);
    // The fixture draws no widgets; the click puts the pointer on its window.
    click_center(&host, &main_display(), window).await;
    wait_for("the click", || mouse_clicks(&f).len() == 1).await;
    // With no window manager nothing focuses a clicked window, and winit
    // drops keys until its window gets FocusIn: do the window manager's part.
    {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConnectionExt, InputFocus};
        let (c, screen) = x11rb::connect(None).unwrap();
        let root = c.setup().roots[screen].root;
        let under = c.query_pointer(root).unwrap().reply().unwrap().child;
        c.set_input_focus(InputFocus::POINTER_ROOT, under, x11rb::CURRENT_TIME)
            .unwrap();
        assert_eq!(c.get_input_focus().unwrap().reply().unwrap().focus, under);
    }
    // 19 distinct characters missing from the map, more than Xvfb's 15 free
    // keycodes: the second segment rebinds them, and must arrive as typed.
    let text = "ação €ł日本語中文한국어ñöüßøåæ";
    host.type_text(&format!("{text}\n")).await.unwrap();
    let want = format!("{text}\\r");
    wait_for("the typed text", || typed(&f) == want).await;
    assert_eq!(typed(&f), want, "{:?}", f.lines());
}

/// Spec risk 6: one find attempt builds a provider (two D-Bus connections
/// and an X one). Prints the cost; the bound only catches a pathology.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore]
async fn snapshot_cost_is_measured() {
    let f = Fixture::start().await;
    let s = session();
    let n: u32 = 20;
    let start = Instant::now();
    for _ in 0..n {
        find(&s, f.pid(), "button[name='Remover']").await;
    }
    let per = start.elapsed() / n;
    eprintln!("linux snapshot cost: {per:?} per find attempt");
    assert!(per < Duration::from_millis(500), "{per:?}");
}
