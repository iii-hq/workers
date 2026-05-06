//! `auth::rbac::*` — HMAC API keys + workspace roles
//! (owner/admin/member/viewer). Graduated from roster/workers/auth (TS) in P4.
//!
//! Distinct from `auth-credentials` (provider token vault); the two crates share
//! no state and their function-id namespaces (`auth::*` vs `auth::rbac::*`) are
//! disjoint by construction.

pub const SKILL_ID: &str = "auth-rbac";
pub const SKILL_MD: &str = include_str!("../skill.md");

pub const SUB_SKILLS: &[(&str, &str)] = &[
    (
        "auth-rbac/workspace_create",
        include_str!("../skills/workspace_create.md"),
    ),
    (
        "auth-rbac/workspace_get",
        include_str!("../skills/workspace_get.md"),
    ),
    (
        "auth-rbac/key_create",
        include_str!("../skills/key_create.md"),
    ),
    ("auth-rbac/key_list", include_str!("../skills/key_list.md")),
    (
        "auth-rbac/key_revoke",
        include_str!("../skills/key_revoke.md"),
    ),
    ("auth-rbac/verify", include_str!("../skills/verify.md")),
    (
        "auth-rbac/role_grant",
        include_str!("../skills/role_grant.md"),
    ),
    (
        "auth-rbac/role_check",
        include_str!("../skills/role_check.md"),
    ),
    (
        "auth-rbac/role_list",
        include_str!("../skills/role_list.md"),
    ),
];

pub mod hmac;
pub mod register;
pub mod roles;
pub mod store;

pub use register::{register_with_iii, AuthRbacConfig, AuthRbacFunctionRefs};
pub use roles::{role_satisfies, Role};
pub use store::{ApiKey, RoleGrant, Workspace};
