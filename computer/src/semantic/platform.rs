//! The real accessibility providers. A fresh provider per call: the executor
//! builds one per snapshot on its own thread (COM is initialised there on
//! Windows), and dropping it frees xa11y's handle cache.

use std::sync::Arc;

use xa11y_core::Provider;

use super::ProviderFactory;

pub fn factory() -> ProviderFactory {
    Arc::new(|| {
        crate::driver::native::pin_process_dpi();
        #[cfg(target_os = "macos")]
        let provider = xa11y_macos::MacOSProvider::new()?;
        #[cfg(target_os = "windows")]
        let provider = xa11y_windows::WindowsProvider::new()?;
        Ok(Arc::new(provider) as Arc<dyn Provider>)
    })
}
