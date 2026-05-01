//! `auth::rbac::*` — HMAC API keys + workspace roles
//! (owner/admin/member/viewer). Graduated from roster/workers/auth (TS) in P4.
//!
//! Distinct from `auth-credentials` (provider token vault); the two crates share
//! no state and their function-id namespaces (`auth::*` vs `auth::rbac::*`) are
//! disjoint by construction.

pub mod hmac;
pub mod register;
pub mod roles;
pub mod store;

pub use register::{register_with_iii, AuthRbacConfig, AuthRbacFunctionRefs};
pub use roles::{role_satisfies, Role};
pub use store::{ApiKey, RoleGrant, Workspace};
