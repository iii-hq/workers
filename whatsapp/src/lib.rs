//! Native WhatsApp functions for the iii engine.
pub mod calls;
pub mod config;
pub mod groups;
pub mod logging;
pub mod messaging;
pub mod registry;
pub mod service;
pub mod sessions;
pub mod validate;

pub fn registry() -> registry::Registry {
    let mut registry = registry::Registry::default();
    sessions::register(&mut registry);
    groups::register(&mut registry);
    messaging::register(&mut registry);
    calls::register(&mut registry);
    registry
}
