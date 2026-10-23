//! macOS accessibility backend using AXUIElement API.

#[cfg(target_os = "macos")]
mod ax;
#[cfg(target_os = "macos")]
mod input;
#[cfg(target_os = "macos")]
mod screenshot;

#[cfg(target_os = "macos")]
pub use ax::accessibility_grant_instructions;
#[cfg(target_os = "macos")]
pub use ax::MacOSProvider;
#[cfg(target_os = "macos")]
pub use input::MacOSInputProvider;
#[cfg(target_os = "macos")]
pub use screenshot::MacOSScreenshot;

#[cfg(not(target_os = "macos"))]
mod stub {
    use xa11y_core::input::{InputProvider, Key, MouseButton, Point, ScrollDelta};
    use xa11y_core::*;

    pub struct MacOSProvider;

    impl MacOSProvider {
        pub fn new() -> Result<Self> {
            Err(Error::Platform {
                code: -1,
                message: "macOS backend only available on macOS".to_string(),
            })
        }
    }

    #[derive(Default)]
    pub struct MacOSInputProvider;

    impl MacOSInputProvider {
        pub fn new() -> Result<Self> {
            Err(Error::Platform {
                code: -1,
                message: "macOS input backend only available on macOS".to_string(),
            })
        }
    }

    pub struct MacOSScreenshot;

    impl MacOSScreenshot {
        pub fn new() -> Result<Self> {
            Err(Error::Platform {
                code: -1,
                message: "macOS screenshot backend only available on macOS".to_string(),
            })
        }
    }

    impl ScreenshotProvider for MacOSScreenshot {
        fn capture_full(&self) -> Result<(Screenshot, Point)> {
            unreachable!()
        }
        fn capture_region(&self, _: Rect) -> Result<Screenshot> {
            unreachable!()
        }
    }

    impl InputProvider for MacOSInputProvider {
        fn pointer_move(&self, _: Point) -> Result<()> {
            unreachable!()
        }
        fn pointer_down(&self, _: MouseButton) -> Result<()> {
            unreachable!()
        }
        fn pointer_up(&self, _: MouseButton) -> Result<()> {
            unreachable!()
        }
        fn pointer_click(&self, _: Point, _: MouseButton, _: u32) -> Result<()> {
            unreachable!()
        }
        fn pointer_scroll(&self, _: Point, _: ScrollDelta) -> Result<()> {
            unreachable!()
        }
        fn key_down(&self, _: &Key) -> Result<()> {
            unreachable!()
        }
        fn key_up(&self, _: &Key) -> Result<()> {
            unreachable!()
        }
        fn type_text(&self, _: &str) -> Result<()> {
            unreachable!()
        }
    }

    impl Provider for MacOSProvider {
        fn get_children(&self, _: Option<&ElementData>) -> Result<Vec<ElementData>> {
            unreachable!()
        }
        fn get_parent(&self, _: &ElementData) -> Result<Option<ElementData>> {
            unreachable!()
        }
        fn list_apps(&self) -> Result<Vec<ElementData>> {
            unreachable!()
        }
        fn focused_app(&self) -> Result<ElementData> {
            unreachable!()
        }
        fn list_shell_surfaces(&self) -> Result<Vec<(ShellSurfaceKind, ElementData)>> {
            unreachable!()
        }
        fn press(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn focus(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn blur(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn toggle(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn select(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn expand(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn collapse(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn show_menu(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn increment(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn decrement(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn scroll_into_view(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn set_value(&self, _: &ElementData, _: &str) -> Result<()> {
            unreachable!()
        }
        fn set_numeric_value(&self, _: &ElementData, _: f64) -> Result<()> {
            unreachable!()
        }
        fn type_text(&self, _: &ElementData, _: &str) -> Result<()> {
            unreachable!()
        }
        fn set_text_selection(&self, _: &ElementData, _: u32, _: u32) -> Result<()> {
            unreachable!()
        }
        fn perform_action(&self, _: &ElementData, _: &str) -> Result<()> {
            unreachable!()
        }
        fn activate(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn minimize(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn maximize(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn enter_fullscreen(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn restore(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn close(&self, _: &ElementData) -> Result<()> {
            unreachable!()
        }
        fn move_to(&self, _: &ElementData, _: i32, _: i32) -> Result<()> {
            unreachable!()
        }
        fn resize_to(&self, _: &ElementData, _: u32, _: u32) -> Result<()> {
            unreachable!()
        }
        fn subscribe(&self, _: &ElementData) -> Result<Subscription> {
            unreachable!()
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub use stub::{MacOSInputProvider, MacOSProvider, MacOSScreenshot};

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use xa11y_core::*;

    #[test]
    fn create_provider() {
        let result = MacOSProvider::new();
        #[cfg(target_os = "macos")]
        assert!(result.is_ok());
        #[cfg(not(target_os = "macos"))]
        assert!(result.is_err());
    }
}
