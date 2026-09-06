//! provider-elevenlabs: ElevenLabs speech behind llm-router. Speech to text
//! with Scribe and text to speech with the Eleven voices, served as
//! `provider::elevenlabs::transcribe` / `provider::elevenlabs::speak`
//! behind `router::transcribe` / `router::speak`. No chat surface: the
//! provider declares only speech models, so chat routing never lands here.

pub mod discovery;
pub mod errors;
pub mod manifest;
pub mod register;
pub mod router_client;
pub mod speech;
pub mod state;
pub mod surface;

/// The provider id — also the `provider::<id>::*` function prefix and the
/// router config slice key.
pub const PROVIDER_ID: &str = "elevenlabs";
