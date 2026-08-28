//! The embedded identity prompt. `DEFAULT` is the single identity EVERY
//! agent gets — top-level sessions and spawned children alike: the basic
//! engine surface and how to discover functions (list, info, call). Roles
//! differ by policy and by the enrich layers (mode paragraph, agent profile,
//! caller prompt), never by a separate embedded identity.

pub const DEFAULT: &str = include_str!("../../prompts/default.txt");
