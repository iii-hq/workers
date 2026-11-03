//! Runs a test again in a child process whose HOME is a scratch directory.
//!
//! The test's own SDK client writes an `iii ps` marker under
//! `$HOME/.iii/run` (the SDK reads nothing else to place it). A test that
//! fails before `shutdown_async` leaves the marker behind, and under the real
//! HOME that is ~/.iii/run.

use browser::chromium::{self, SearchEnv, Source};
use browser::config::WorkerConfig;

const CHILD_ENV: &str = "BROWSER_TEST_PRIVATE_HOME";

/// `true` in the child, which goes on with the test. The parent runs the
/// calling test in the child, fails when the child fails, and returns
/// `false`, so the caller stops.
pub fn in_child() -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return true;
    }
    // libtest runs each test on a thread named after it.
    let test = std::thread::current()
        .name()
        .filter(|name| *name != "main")
        .expect("a libtest thread named after the test")
        .to_owned();
    let home = std::env::temp_dir().join(format!(
        "browser-test-home-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&home).expect("create the scratch HOME");
    let real = SearchEnv::current(&WorkerConfig::default());
    // Only a system Chrome under HOME (macOS ~/Applications) is out of the child's sight.
    let home_chrome = real.resolve().filter(|found| {
        found.source == Source::System
            && std::env::var_os("HOME").is_some_and(|h| found.path.starts_with(h))
    });
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", &test, "--include-ignored", "--nocapture"])
        .env("HOME", &home)
        .env(CHILD_ENV, "1")
        // The real caches, not CHROME: the source decides whether Chromium runs with --no-sandbox.
        .env(chromium::CACHE_DIR_ENV, chromium::cache_root())
        .envs(
            real.playwright_dir
                .map(|dir| ("PLAYWRIGHT_BROWSERS_PATH", dir)),
        )
        .envs(real.puppeteer_dir.map(|dir| ("PUPPETEER_CACHE_DIR", dir)))
        .envs(home_chrome.map(|found| ("CHROME", found.path)))
        .output()
        .expect("run the test in a private HOME");
    let _ = std::fs::remove_dir_all(&home);
    eprint!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "{test} failed in its private-HOME child"
    );
    false
}
