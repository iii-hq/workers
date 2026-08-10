//! The error taxonomy. Every variant maps to a stable
//! `node-engine::<snake_case>` wire code; the hosting worker converts into
//! its SDK error so the engine surfaces `code: message` to callers.
//!
//! The `From<NodeEngineError> for iii_sdk::errors::Error` impl that used to
//! live here is gone with this crate's SDK dependency — with both types
//! foreign to the worker, the orphan rule forbids it there. The worker
//! supplies a plain function instead, whose output is byte-identical
//! (`Error::Handler(e.to_string())`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeEngineError {
    /// The evaluated code threw, or its promise rejected.
    ///
    /// Carries the logs captured BEFORE the throw. A host that reports a
    /// failing script as a normal process-shaped response — non-zero exit
    /// code, traceback on stderr — needs its stdout verbatim, and this used
    /// to be discarded at both failure sites in `runtime.rs`.
    EvalFailed {
        message: String,
        logs: Vec<crate::runtime::LogLine>,
    },
    /// An eval or handler invocation blew its deadline. The isolate is killed.
    Timeout,
    /// The isolate hit its V8 heap cap and was killed.
    Oom,
    RuntimeNotFound(String),
    /// The runtime died (timeout/OOM/shutdown) while a call was in flight.
    RuntimeGone(String),
    /// `max_runtimes` live isolates already exist.
    Capacity(usize),
    NamespaceDenied {
        id: String,
        namespace: String,
    },
    /// Another runtime — or this worker itself — already registered this id.
    /// Deliberately carries only the id: the owning `runtime_id` is a
    /// capability and must never be disclosed to a caller who does not hold it.
    IdTaken(String),
    InvalidRequest(String),
}

impl NodeEngineError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EvalFailed { .. } => "node-engine::eval_failed",
            Self::Timeout => "node-engine::timeout",
            Self::Oom => "node-engine::oom",
            Self::RuntimeNotFound(_) => "node-engine::runtime_not_found",
            Self::RuntimeGone(_) => "node-engine::runtime_gone",
            Self::Capacity(_) => "node-engine::capacity",
            Self::NamespaceDenied { .. } => "node-engine::namespace_denied",
            Self::IdTaken(_) => "node-engine::id_taken",
            Self::InvalidRequest(_) => "node-engine::invalid_request",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::EvalFailed { message, .. } => message.clone(),
            Self::Timeout => "execution exceeded its deadline; the runtime was terminated".into(),
            Self::Oom => "heap limit exceeded; the runtime was terminated".into(),
            Self::RuntimeNotFound(id) => format!("unknown runtime_id {id}"),
            Self::RuntimeGone(id) => format!("runtime {id} is no longer alive"),
            Self::Capacity(max) => format!("all {max} runtime slots are in use"),
            // Shared by both `op_iii_register` and `op_iii_unregister` (see
            // ops.rs), so the wording stays neutral between them rather than
            // prescribing a register-only fix like "rename the id" — reusing
            // one variant for both is what makes "not yours" and "not found"
            // the same answer on the unregister path.
            Self::NamespaceDenied { id, namespace } => format!(
                "id {id:?} must start with this runtime's namespace {namespace:?} — a runtime \
                 can only register or unregister ids inside its own namespace"
            ),
            // Shared by function AND trigger-type registration (both live in
            // the same namespaced id space), so the wording names neither.
            Self::IdTaken(id) => format!("id {id} is already registered"),
            Self::InvalidRequest(m) => m.clone(),
        }
    }
}

impl NodeEngineError {
    /// An eval failure with no logs — the compile-error and envelope-decode
    /// paths, where nothing tenant-authored ran.
    pub fn eval_failed(message: impl Into<String>) -> Self {
        Self::EvalFailed {
            message: message.into(),
            logs: Vec::new(),
        }
    }

    /// Attach the logs an eval produced before it failed. A no-op on every
    /// other variant, so a call site can apply it unconditionally.
    pub fn with_logs(self, logs: Vec<crate::runtime::LogLine>) -> Self {
        match self {
            Self::EvalFailed { message, .. } => Self::EvalFailed { message, logs },
            other => other,
        }
    }
}

impl std::fmt::Display for NodeEngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for NodeEngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_wire_strings() {
        assert_eq!(
            NodeEngineError::eval_failed("x").code(),
            "node-engine::eval_failed"
        );
        assert_eq!(NodeEngineError::Timeout.code(), "node-engine::timeout");
        assert_eq!(NodeEngineError::Oom.code(), "node-engine::oom");
        assert_eq!(
            NodeEngineError::RuntimeNotFound("r".into()).code(),
            "node-engine::runtime_not_found"
        );
        assert_eq!(
            NodeEngineError::RuntimeGone("r".into()).code(),
            "node-engine::runtime_gone"
        );
        assert_eq!(NodeEngineError::Capacity(3).code(), "node-engine::capacity");
        assert_eq!(
            NodeEngineError::NamespaceDenied {
                id: "state::get".into(),
                namespace: "ns::".into()
            }
            .code(),
            "node-engine::namespace_denied"
        );
        assert_eq!(
            NodeEngineError::IdTaken("app::a".into()).code(),
            "node-engine::id_taken"
        );
        assert_eq!(
            NodeEngineError::InvalidRequest("x".into()).code(),
            "node-engine::invalid_request"
        );
    }

    #[test]
    fn display_is_code_colon_message() {
        let e = NodeEngineError::RuntimeNotFound("rt-7".into());
        assert_eq!(
            e.to_string(),
            "node-engine::runtime_not_found: unknown runtime_id rt-7"
        );
    }
}
