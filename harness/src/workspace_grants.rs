use std::collections::BTreeSet;

use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};

use crate::error::HarnessError;

pub const WORKSPACE_GRANTS_SCOPE: &str = "harness_workspace_grants";

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

pub async fn purge(iii: &IIIClient, session_id: &str, timeout_ms: u64) -> Result<(), HarnessError> {
    crate::state::state_delete(iii, WORKSPACE_GRANTS_SCOPE, session_id, timeout_ms).await
}

async fn read(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<GrantSet, HarnessError> {
    let value =
        crate::state::state_get(iii, WORKSPACE_GRANTS_SCOPE, session_id, timeout_ms).await?;
    if value.is_null() {
        return Ok(GrantSet::default());
    }
    serde_json::from_value(value)
        .map_err(|e| HarnessError::State(format!("workspace grants parse: {e}")))
}

async fn write(
    iii: &IIIClient,
    session_id: &str,
    grants: &GrantSet,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let value = serde_json::to_value(grants)
        .map_err(|e| HarnessError::State(format!("workspace grants serialize: {e}")))?;
    crate::state::state_set(iii, WORKSPACE_GRANTS_SCOPE, session_id, value, timeout_ms).await
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
}
