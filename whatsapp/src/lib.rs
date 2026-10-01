//! Native WhatsApp functions for the iii engine.
pub mod calls;
pub mod cloud;
pub mod config;
pub mod configuration;
pub mod groups;
pub mod inbox;
pub mod logging;
pub mod messaging;
pub mod registry;
pub mod service;
pub mod sessions;
pub mod ui;
pub mod validate;

pub fn registry() -> registry::Registry {
    let mut registry = registry::Registry::default();
    sessions::register(&mut registry);
    groups::register(&mut registry);
    inbox::register(&mut registry);
    messaging::register(&mut registry);
    calls::register(&mut registry);
    cloud::register(&mut registry);
    registry
}
