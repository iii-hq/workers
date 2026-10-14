//! A native session with no coordinate driver: the desktop (a Wayland
//! compositor) gives this worker no screenshots and no synthetic input, but
//! its accessibility tree is still readable and actionable by ref.

use async_trait::async_trait;

use super::{Driver, Screen, Shot};

/// Refuses every capture and input with the reason it was built with. The
/// screen is 0x0: there is no coordinate space to describe.
pub struct SemanticOnly(pub String);

#[async_trait]
impl Driver for SemanticOnly {
    async fn screen_size(&self) -> Result<Screen, String> {
        Ok(Screen {
            width: 0,
            height: 0,
        })
    }
    async fn screenshot(&self) -> Result<Shot, String> {
        Err(self.0.clone())
    }
    async fn left_click(&self, _: i64, _: i64) -> Result<(), String> {
        Err(self.0.clone())
    }
    async fn right_click(&self, _: i64, _: i64) -> Result<(), String> {
        Err(self.0.clone())
    }
    async fn double_click(&self, _: i64, _: i64) -> Result<(), String> {
        Err(self.0.clone())
    }
    async fn move_cursor(&self, _: i64, _: i64) -> Result<(), String> {
        Err(self.0.clone())
    }
    async fn scroll(&self, _: i64, _: i64, _: i64, _: i64) -> Result<(), String> {
        Err(self.0.clone())
    }
    async fn drag(&self, _: (i64, i64), _: (i64, i64), _: &str) -> Result<(), String> {
        Err(self.0.clone())
    }
    async fn type_text(&self, _: &str) -> Result<(), String> {
        Err(self.0.clone())
    }
    async fn keypress(&self, _: &[String]) -> Result<(), String> {
        Err(self.0.clone())
    }
    async fn close(&self) -> Result<(), String> {
        Ok(())
    }
    fn coordinates(&self) -> Option<&str> {
        Some(&self.0)
    }
}
