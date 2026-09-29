//! Worker errors.
//!
//! Every variant renders as `sentinel/<code>: <detail>`. The code is the
//! stable half: the console matches on it to pick its copy, and an agent that
//! called [`crate::functions::DIAGNOSIS_RECORD_ID`] with the wrong group reads
//! it in the function result and corrects itself.

#[derive(Debug, thiserror::Error)]
pub enum SentinelError {
    #[error("sentinel/invalid_request: {0}")]
    InvalidRequest(String),
    #[error("sentinel/not_found: {0}")]
    NotFound(String),
    /// A state change the lifecycle does not allow (resolving a group that is
    /// already ignored, say). Carries both ends so the message names them.
    #[error("sentinel/invalid_transition: a group cannot move from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    /// The calling session is not an investigation: no `iii.session.id` in the
    /// invocation baggage, or an id that names no investigation.
    #[error("sentinel/no_investigation: {0}")]
    NoInvestigation(String),
    /// The calling session investigates a different group.
    #[error("sentinel/not_this_group: {0}")]
    NotThisGroup(String),
    #[error("sentinel/no_model: {0}")]
    NoModel(String),
    #[error("sentinel/no_evidence: {0}")]
    NoEvidence(String),
    #[error("sentinel/harness_unavailable: {0}")]
    HarnessUnavailable(String),
    /// The durable dependencies (schema, queue) are not claimed yet. Callers
    /// retry; the queue redelivers.
    #[error("sentinel/not_ready: {0}")]
    NotReady(String),
    #[error("sentinel/dependency: {0}")]
    Dependency(String),
}

impl SentinelError {
    /// The stable error code, without the `sentinel/` prefix or the detail.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::NotFound(_) => "not_found",
            Self::InvalidTransition { .. } => "invalid_transition",
            Self::NoInvestigation(_) => "no_investigation",
            Self::NotThisGroup(_) => "not_this_group",
            Self::NoModel(_) => "no_model",
            Self::NoEvidence(_) => "no_evidence",
            Self::HarnessUnavailable(_) => "harness_unavailable",
            Self::NotReady(_) => "not_ready",
            Self::Dependency(_) => "dependency",
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    pub fn dependency(message: impl Into<String>) -> Self {
        Self::Dependency(message.into())
    }
}

impl From<SentinelError> for iii_sdk::errors::Error {
    fn from(error: SentinelError) -> Self {
        Self::Handler(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_carries_its_code() {
        for error in [
            SentinelError::invalid("bad"),
            SentinelError::NotFound("grp_x".into()),
            SentinelError::InvalidTransition {
                from: "ignored".into(),
                to: "resolved".into(),
            },
            SentinelError::NoInvestigation("no session".into()),
            SentinelError::NotThisGroup("grp_y".into()),
            SentinelError::NoModel("configure one".into()),
            SentinelError::NoEvidence("occ_x".into()),
            SentinelError::HarnessUnavailable("send failed".into()),
            SentinelError::NotReady("queue".into()),
            SentinelError::dependency("database"),
        ] {
            let rendered = error.to_string();
            assert!(
                rendered.starts_with(&format!("sentinel/{}: ", error.code())),
                "{rendered} does not lead with its code"
            );
        }
    }
}
