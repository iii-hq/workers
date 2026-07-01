//! Function registration for the web worker.

use std::sync::Arc;

use iii_sdk::{IIIClient, RegisterFunction};

use crate::config::SharedConfig;

pub mod fetch;
pub mod inject_guidance;

pub fn register_all(iii: &Arc<IIIClient>, shared: &SharedConfig) {
    fetch::register(iii, shared);

    iii.register_function(
        inject_guidance::GUIDANCE_HOOK_ID,
        RegisterFunction::new_async(move |event: inject_guidance::PreGenerateEvent| async move {
            inject_guidance::handle(event).await
        })
        .description(inject_guidance::GUIDANCE_HOOK_DESC),
    );
}
