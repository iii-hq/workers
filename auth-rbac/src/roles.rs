//! Workspace role hierarchy: owner > admin > member > viewer.
//! Direct port of roster/workers/auth/src/roles.ts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Admin,
    Member,
    Viewer,
}

impl Role {
    fn rank(self) -> u8 {
        match self {
            Self::Owner => 4,
            Self::Admin => 3,
            Self::Member => 2,
            Self::Viewer => 1,
        }
    }
}

pub fn parse_role(s: &str) -> Option<Role> {
    match s {
        "owner" => Some(Role::Owner),
        "admin" => Some(Role::Admin),
        "member" => Some(Role::Member),
        "viewer" => Some(Role::Viewer),
        _ => None,
    }
}

pub fn assert_role(s: &str) -> Result<Role, String> {
    parse_role(s)
        .ok_or_else(|| format!("invalid role: {s}. expected one of owner, admin, member, viewer"))
}

/// `required <= granted`
pub fn role_satisfies(granted: Role, required: Role) -> bool {
    granted.rank() >= required.rank()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_role_recognises_all_four() {
        assert_eq!(parse_role("owner"), Some(Role::Owner));
        assert_eq!(parse_role("admin"), Some(Role::Admin));
        assert_eq!(parse_role("member"), Some(Role::Member));
        assert_eq!(parse_role("viewer"), Some(Role::Viewer));
    }

    #[test]
    fn parse_role_rejects_unknown() {
        assert_eq!(parse_role("god"), None);
        assert_eq!(parse_role(""), None);
    }

    #[test]
    fn assert_role_returns_descriptive_error() {
        let err = assert_role("god").unwrap_err();
        assert!(err.contains("god"));
        assert!(err.contains("owner"));
    }

    #[test]
    fn satisfies_uses_rank_ordering() {
        assert!(role_satisfies(Role::Owner, Role::Viewer));
        assert!(role_satisfies(Role::Admin, Role::Member));
        assert!(role_satisfies(Role::Viewer, Role::Viewer));
        assert!(!role_satisfies(Role::Member, Role::Admin));
        assert!(!role_satisfies(Role::Viewer, Role::Owner));
    }
}
