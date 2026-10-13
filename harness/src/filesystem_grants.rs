use std::collections::BTreeSet;

use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};

use crate::error::HarnessError;

pub const FILESYSTEM_GRANTS_SCOPE: &str = "harness_filesystem_grants";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantSet {
    #[serde(default)]
    roots: BTreeSet<String>,
}

impl GrantSet {
    pub fn grant(&mut self, root: String) -> bool {
        self.roots.insert(root)
    }

    pub fn revoke(&mut self, root: &str) -> bool {
        self.roots.remove(root)
    }

    pub fn roots(&self) -> Vec<String> {
        self.roots.iter().cloned().collect()
    }
}

pub async fn roots(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Vec<String>, HarnessError> {
    Ok(read(iii, session_id, timeout_ms).await?.roots())
}

pub async fn grant(
    iii: &IIIClient,
    session_id: &str,
    root: String,
    timeout_ms: u64,
) -> Result<Vec<String>, HarnessError> {
    let mut grants = read(iii, session_id, timeout_ms).await?;
    grants.grant(root);
    write(iii, session_id, &grants, timeout_ms).await?;
    Ok(grants.roots())
}

pub async fn revoke(
    iii: &IIIClient,
    session_id: &str,
    root: &str,
    timeout_ms: u64,
) -> Result<Vec<String>, HarnessError> {
    let mut grants = read(iii, session_id, timeout_ms).await?;
    grants.revoke(root);
    write(iii, session_id, &grants, timeout_ms).await?;
    Ok(grants.roots())
}

/// A spawned child starts with its parent's grants. A snapshot: later grants
/// to the parent do not propagate.
pub async fn copy(
    iii: &IIIClient,
    from_session_id: &str,
    to_session_id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let parent = read(iii, from_session_id, timeout_ms).await?;
    if parent.roots.is_empty() {
        return Ok(());
    }
    let child = merged(&parent, read(iii, to_session_id, timeout_ms).await?);
    write(iii, to_session_id, &child, timeout_ms).await
}

fn merged(parent: &GrantSet, mut child: GrantSet) -> GrantSet {
    child.roots.extend(parent.roots.iter().cloned());
    child
}

pub async fn purge(iii: &IIIClient, session_id: &str, timeout_ms: u64) -> Result<(), HarnessError> {
    crate::state::state_delete(iii, FILESYSTEM_GRANTS_SCOPE, session_id, timeout_ms).await
}

async fn read(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<GrantSet, HarnessError> {
    let value =
        crate::state::state_get(iii, FILESYSTEM_GRANTS_SCOPE, session_id, timeout_ms).await?;
    if value.is_null() {
        return Ok(GrantSet::default());
    }
    serde_json::from_value(value)
        .map_err(|e| HarnessError::State(format!("filesystem grants parse: {e}")))
}

async fn write(
    iii: &IIIClient,
    session_id: &str,
    grants: &GrantSet,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let value = serde_json::to_value(grants)
        .map_err(|e| HarnessError::State(format!("filesystem grants serialize: {e}")))?;
    crate::state::state_set(iii, FILESYSTEM_GRANTS_SCOPE, session_id, value, timeout_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_set_dedupes_lists_and_revokes_session_roots() {
        let mut grants = GrantSet::default();
        grants.grant("/tmp/z".to_string());
        grants.grant("/tmp/a".to_string());
        grants.grant("/tmp/z".to_string());

        assert_eq!(
            grants.roots(),
            vec!["/tmp/a".to_string(), "/tmp/z".to_string()]
        );
        assert!(grants.revoke("/tmp/z"));
        assert!(!grants.revoke("/tmp/missing"));
        assert_eq!(grants.roots(), vec!["/tmp/a".to_string()]);
    }

    #[test]
    fn a_child_starts_with_its_parents_grants() {
        let mut parent = GrantSet::default();
        parent.grant("/a".into());
        parent.grant("/b".into());
        let mut child = GrantSet::default();
        child.grant("/b".into());
        child.grant("/c".into());
        assert_eq!(
            merged(&parent, child).roots(),
            vec!["/a".to_string(), "/b".to_string(), "/c".to_string()]
        );
    }
}
