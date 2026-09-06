//! The embedded identity prompt. `DEFAULT` is the single identity EVERY
//! agent gets — top-level sessions and spawned children alike: the basic
//! engine surface and how to discover functions (list, info, call). Roles
//! differ by policy, by the enrich layer (caller prompt) or by an agent
//! profile whose resolved prompt replaces this one (the
//! directory's bundled `iii` profile carries this same text as the base
//! profiles extend), never by a separate embedded identity.

pub const DEFAULT: &str = include_str!("../../prompts/default.txt");
