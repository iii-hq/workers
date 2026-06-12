//! Wire contracts from tech-specs/2026-06-agentic/README.md § Cross-cutting
//! contracts. Types only — no logic. Channel refs are `iii_sdk`'s own
//! `StreamChannelRef`; everything else is defined here.

pub mod content;
pub mod credential;
pub mod errors;
pub mod events;
pub mod messages;
pub mod model;
pub mod router;

pub use self::content::*;
pub use self::credential::*;
pub use self::errors::*;
pub use self::events::*;
pub use self::messages::*;
pub use self::model::*;
pub use self::router::*;
