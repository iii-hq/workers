//! Shared test doubles: a fake desktop that counts what it was asked to do,
//! and `Session` builders for handler and lifecycle tests.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::driver::{Driver, Screen, Shot};
use crate::frames::{FrameNotifier, FrameSink, FrameTarget};
use crate::semantic::SemanticSession;
use crate::session::{now_ms, Session};

/// Every screenshot is a distinct ~64 KB JPEG-looking buffer; `fail` makes
/// captures error; `inputs` counts every pointer/keyboard call on entry;
/// `hold` keeps each input call pending until cleared; `fail_input` makes
/// every input fail after it was counted.
pub(crate) struct FakeDesktop {
    pub shots: AtomicU64,
    pub fail: AtomicBool,
    pub inputs: AtomicU64,
    pub hold: AtomicBool,
    pub fail_input: AtomicBool,
}

impl FakeDesktop {
    async fn input(&self) -> Result<(), String> {
        self.inputs.fetch_add(1, Ordering::SeqCst);
        while self.hold.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        if self.fail_input.load(Ordering::SeqCst) {
            return Err("input failed".to_string());
        }
        Ok(())
    }
}

#[async_trait]
impl Driver for FakeDesktop {
    async fn screen_size(&self) -> Result<Screen, String> {
        Ok(Screen {
            width: 1280,
            height: 800,
        })
    }
    async fn screenshot(&self) -> Result<Shot, String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("display went away".to_string());
        }
        let n = self.shots.fetch_add(1, Ordering::SeqCst);
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
        bytes.extend(std::iter::repeat_n((n % 251) as u8, 64 * 1024));
        Ok(Shot::new(bytes))
    }
    async fn left_click(&self, _: i64, _: i64) -> Result<(), String> {
        self.input().await
    }
    async fn right_click(&self, _: i64, _: i64) -> Result<(), String> {
        self.input().await
    }
    async fn double_click(&self, _: i64, _: i64) -> Result<(), String> {
        self.input().await
    }
    async fn move_cursor(&self, _: i64, _: i64) -> Result<(), String> {
        self.input().await
    }
    async fn scroll(&self, _: i64, _: i64, _: i64, _: i64) -> Result<(), String> {
        self.input().await
    }
    async fn drag(&self, _: (i64, i64), _: (i64, i64), _: &str) -> Result<(), String> {
        self.input().await
    }
    async fn type_text(&self, _: &str) -> Result<(), String> {
        self.input().await
    }
    async fn keypress(&self, _: &[String]) -> Result<(), String> {
        self.input().await
    }
    async fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

/// A frame sink that drops everything.
pub(crate) struct NullSink;

#[async_trait]
impl FrameSink for NullSink {
    async fn deliver(&self, _: &FrameTarget, _: Value) -> Result<(), String> {
        Ok(())
    }
}

pub(crate) fn session_on(
    notifier: &Arc<FrameNotifier>,
    id: &str,
    semantic: Option<SemanticSession>,
) -> (Arc<Session>, Arc<FakeDesktop>) {
    let desktop = Arc::new(FakeDesktop {
        shots: AtomicU64::new(0),
        fail: AtomicBool::new(false),
        inputs: AtomicU64::new(0),
        hold: AtomicBool::new(false),
        fail_input: AtomicBool::new(false),
    });
    let cfg = crate::config::WorkerConfig {
        screencast_fps: 200, // 5 ms frames keep the tests fast
        ..Default::default()
    };
    let session = Session::new(
        id.to_string(),
        "fake".to_string(),
        "linux".to_string(),
        Screen {
            width: 1280,
            height: 800,
        },
        now_ms(),
        desktop.clone(),
        cfg.into_shared(),
        notifier.clone(),
        semantic,
    );
    (session, desktop)
}

pub(crate) fn session_with(semantic: Option<SemanticSession>) -> (Arc<Session>, Arc<FakeDesktop>) {
    session_on(&FrameNotifier::new(Arc::new(NullSink)), "c1", semantic)
}

/// The wire code of a semantic refusal, or "handler: …" for any other error.
pub(crate) fn code(e: iii_sdk::errors::Error) -> String {
    match e {
        iii_sdk::errors::Error::Remote { code, .. } => code,
        other => format!("handler: {other}"),
    }
}
