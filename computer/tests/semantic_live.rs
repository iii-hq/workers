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
#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use computer::driver::{DisplayInfo, Driver, NativeHost};
use computer::semantic::{parse_selector, platform, Code, Query, Scope, SemanticSession, Verb};

const SECRET: &str = "s3cr3t-fixture";

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
    async fn start() -> Self {
        let bin = std::env::var("COMPUTER_A11Y_FIXTURE")
            .expect("set COMPUTER_A11Y_FIXTURE to the built computer-a11y-fixture binary");
        static RUN: AtomicU64 = AtomicU64::new(0);
        let log = std::env::temp_dir().join(format!(
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
        // AccessKit serves an empty placeholder to the first accessibility
        // query and builds the real tree on its event loop: wait until the
        // tree is really there instead of sleeping a guess.
        let probe = session();
        let start = Instant::now();
        while probe
            .snapshot(query(f.pid(), Some("button[name='Remover']")), false)
            .await
            .map(|o| o.matches)
            != Ok(1)
        {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "fixture tree never appeared"
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

    fn actions(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|l| l.starts_with("action "))
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

fn ref_on(text: &str, needle: &str) -> String {
    let line = text
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle} in:\n{text}"));
    refs(line).remove(0)
}

fn settle() {
    std::thread::sleep(Duration::from_millis(400));
}

async fn find(s: &SemanticSession, pid: u32, selector: &str) -> computer::semantic::SnapshotOut {
    s.snapshot(query(pid, Some(selector)), false).await.unwrap()
}

#[tokio::test]
#[ignore]
async fn invoke_dispatches_exactly_once() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "button[name='Salvar']").await;
    assert_eq!(out.matches, 1, "{}", out.text);
    s.act(refs(&out.text).remove(0), Verb::Invoke, None)
        .await
        .unwrap();
    settle();
    let actions = f.actions();
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert!(actions[0].starts_with("action Salvar Click"), "{actions:?}");
    assert_eq!(
        find(&s, f.pid(), "[name^='Status: salvo 1']").await.matches,
        1
    );
}

#[tokio::test]
#[ignore]
async fn set_value_round_trips_unicode() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "text_field[name='Nome']").await;
    s.act(
        refs(&out.text).remove(0),
        Verb::SetValue,
        Some("ação, João, €".into()),
    )
    .await
    .unwrap();
    settle();
    assert_eq!(f.actions().len(), 1, "{:?}", f.actions());
    assert_eq!(
        find(
            &s,
            f.pid(),
            "text_field[name='Nome'][value='ação, João, €']"
        )
        .await
        .matches,
        1
    );
}

#[tokio::test]
#[ignore]
async fn toggle_and_select_change_observed_state() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "check_box[name='Ativo']").await;
    s.act(refs(&out.text).remove(0), Verb::Toggle, None)
        .await
        .unwrap();
    settle();
    // Counts, not names: how UIA/AX toggle and select map onto AccessKit
    // actions is the platform's business; one request per act is ours.
    assert_eq!(f.actions().len(), 1, "{:?}", f.actions());
    assert_eq!(
        find(&s, f.pid(), "check_box[name='Ativo'][checked='on']")
            .await
            .matches,
        1
    );
    let out = find(&s, f.pid(), "[name='Dois']").await;
    s.act(refs(&out.text).remove(0), Verb::Select, None)
        .await
        .unwrap();
    settle();
    assert_eq!(f.actions().len(), 2, "{:?}", f.actions());
    assert_eq!(
        find(&s, f.pid(), "[name='Dois'][selected='true']")
            .await
            .matches,
        1
    );
}

#[tokio::test]
#[ignore]
async fn duplicates_are_listed_never_picked() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "button[name='OK']").await;
    assert_eq!(out.matches, 2, "{}", out.text);
    settle();
    assert!(f.actions().is_empty());
}

#[tokio::test]
#[ignore]
async fn a_disabled_button_is_refused_without_effect() {
    let f = Fixture::start().await;
    let s = session();
    let out = find(&s, f.pid(), "button[name='Desabilitado']").await;
    let err = s
        .act(refs(&out.text).remove(0), Verb::Invoke, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, Code::NotSupported);
    settle();
    assert!(f.actions().is_empty());
    assert_eq!(
        find(&s, f.pid(), "[name^='Status: salvo 0']").await.matches,
        1
    );
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
    settle();
    let before = f.actions().len();
    let err = s.act(salvar, Verb::Invoke, None).await.unwrap_err();
    assert_eq!(err.code, Code::StaleRef, "{}", err.message);
    settle();
    assert_eq!(f.actions().len(), before);
}

#[tokio::test]
#[ignore]
async fn a_replaced_snapshot_is_a_stale_ref() {
    let f = Fixture::start().await;
    let s = session();
    let old = refs(&find(&s, f.pid(), "button[name='Salvar']").await.text).remove(0);
    find(&s, f.pid(), "button[name='Remover']").await;
    assert_eq!(
        s.act(old, Verb::Invoke, None).await.unwrap_err().code,
        Code::StaleRef
    );
    settle();
    assert!(f.actions().is_empty());
}

#[tokio::test]
#[ignore]
async fn the_password_value_never_leaves() {
    let f = Fixture::start().await;
    let s = session();
    let tree = s.snapshot(query(f.pid(), None), true).await.unwrap();
    assert!(tree.text.contains("Senha"), "{}", tree.text);
    assert!(!tree.text.contains(SECRET));
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
    settle();
    assert!(f.actions().is_empty());
}

/// Click the centre of the fixture window through the coordinate driver,
/// mapping screen coordinates into the session's (downscaled) image space.
/// The fixture logs physical pixels; `list_displays` sizes are what `xcap`
/// reports — physical on Windows (per-monitor-v2), logical points on macOS.
async fn click_center(host: &NativeHost, display: &DisplayInfo, win: Win) {
    host.screenshot().await.unwrap();
    let image = host.screen_size().await.unwrap();
    let s = if cfg!(target_os = "macos") {
        win.scale
    } else {
        1.0
    };
    let tx = ((win.x + win.w / 2) as f64 / s) as i64;
    let ty = ((win.y + win.h / 2) as f64 / s) as i64;
    let ix = tx * image.width as i64 / display.width as i64;
    let iy = ty * image.height as i64 / display.height as i64;
    host.left_click(ix, iy).await.unwrap();
}

/// Smoke test: a coordinate click on the fixture lands, and still lands after
/// a semantic call in the same process.
#[tokio::test]
#[ignore]
async fn a_coordinate_click_lands_before_and_after_semantic_use() {
    let f = Fixture::start().await;
    let window = f.window().unwrap();
    let (ww, wh) = (window.w, window.h);
    let host = NativeHost::new(4096, 80, None);
    let display = computer::driver::native::list_displays()
        .unwrap()
        .into_iter()
        .find(|d| d.primary)
        .expect("a primary display");
    click_center(&host, &display, window).await;
    settle();
    let s = session();
    s.snapshot(query(f.pid(), None), true).await.unwrap();
    click_center(&host, &display, window).await;
    settle();
    let clicks: Vec<(i64, i64)> = f
        .lines()
        .into_iter()
        .filter_map(|l| l.strip_prefix("mouse ").map(String::from))
        .map(|l| {
            let v: Vec<i64> = l.split(' ').map(|n| n.parse().unwrap()).collect();
            (v[0], v[1])
        })
        .collect();
    assert_eq!(clicks.len(), 2, "{:?}", f.lines());
    for (cx, cy) in clicks {
        assert!(
            (cx - ww / 2).abs() <= 2 && (cy - wh / 2).abs() <= 2,
            "click at ({cx}, {cy}) in {ww}x{wh}"
        );
    }
}
