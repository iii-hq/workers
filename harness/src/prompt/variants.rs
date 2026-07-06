//! The embedded fallback identity prompt (engine-grounded capability ladder).
//! Provider-specific variants live in each provider worker and are served via
//! `router::system_prompt::get`.

pub const DEFAULT: &str = include_str!("../../prompts/default.txt");
