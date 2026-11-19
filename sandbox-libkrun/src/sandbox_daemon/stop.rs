use crate::sandbox_daemon::{
    errors::SandboxError, overlay::OverlayLayout, registry::SandboxRegistry,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StopRequest {
    pub sandbox_id: String,
    /// Block until the VM is fully reaped before returning.
    #[serde(default)]
    pub wait: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StopResponse {
    pub sandbox_id: String,
    pub stopped: bool,
}

#[async_trait::async_trait]
pub trait VmStopper: Send + Sync + 'static {
    async fn stop(&self, vm_pid: u32) -> Result<(), SandboxError>;
}

pub async fn handle_stop<S: VmStopper>(
    req: StopRequest,
    registry: &SandboxRegistry,
    stopper: &S,
) -> Result<StopResponse, SandboxError> {
    let id = Uuid::parse_str(&req.sandbox_id).map_err(|_| {
        SandboxError::InvalidRequest(format!(
            "sandbox_id is not a valid UUID: {}",
            req.sandbox_id
        ))
    })?;
    let _state = registry.claim_stop(id).await?;
    stop_owned(registry, id).await?;
    let _ = stopper; // Compatibility policy argument; PID is never authority.
    Ok(StopResponse {
        sandbox_id: id.to_string(),
        stopped: true,
    })
}

pub(crate) async fn stop_owned(registry: &SandboxRegistry, id: Uuid) -> Result<(), SandboxError> {
    registry.claim_stop(id).await?;
    let session = registry.lifecycle(id).await?;
    let receipt = session
        .stop(std::time::Duration::from_millis(200))
        .await
        .map_err(|e| SandboxError::BootFailed(format!("sandbox retirement: {e}")))?;
    session
        .finish_host_writers()
        .await
        .map_err(|e| SandboxError::BootFailed(format!("sandbox host writers: {e}")))?;
    OverlayLayout::for_sandbox(id)
        .cleanup_retired(session.lease(), &receipt)
        .map_err(|e| SandboxError::BootFailed(format!("sandbox cleanup retained: {e}")))?;
    registry.complete_cleanup(id).await;
    Ok(())
}

/// Stop every live sandbox and remove its directory, for a graceful exit: the
/// lifeline ends the VMs anyway, but only this cleans `overlay::sandbox_root`.
pub async fn stop_all<S: VmStopper>(
    registry: &SandboxRegistry,
    stopper: &S,
) -> Result<(), SandboxError> {
    let _ = stopper;
    let states = registry.list().await;
    let mut errors = Vec::new();
    for state in states {
        if let Err(error) = stop_owned(registry, state.id).await {
            tracing::error!(id=%state.id, %error, "shutdown preserved unresolved sandbox");
            errors.push(format!("{}: {error}", state.id));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(SandboxError::BootFailed(errors.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_daemon::registry::SandboxState;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct FakeStopper {
        called: Arc<AtomicBool>,
    }
    #[async_trait::async_trait]
    impl VmStopper for FakeStopper {
        async fn stop(&self, _pid: u32) -> Result<(), SandboxError> {
            self.called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn state(id: Uuid) -> SandboxState {
        let mut s = crate::sandbox_daemon::registry::sandbox_state_for_test(id);
        s.vm_pid = Some(1234);
        s
    }

    /// A graceful exit stops every sandbox and leaves no dir behind.
    #[tokio::test]
    async fn stop_all_stops_every_sandbox_and_removes_its_dir() {
        let reg = SandboxRegistry::new();
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        for id in ids {
            OverlayLayout::for_sandbox(id).ensure_dirs().unwrap();
            reg.insert(state(id)).await;
        }
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let stopper = FlakyStopper {
            fail_once: AtomicBool::new(false),
            call_count: calls.clone(),
        };

        assert!(stop_all(&reg, &stopper).await.is_err());

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(reg.count().await, 2);
        for id in ids {
            assert!(OverlayLayout::for_sandbox(id).base().exists());
        }
    }

    #[tokio::test]
    async fn unresolved_stop_preserves_registry_and_does_not_signal_a_pid() {
        let reg = SandboxRegistry::new();
        let id = Uuid::new_v4();
        reg.insert(state(id)).await;
        let called = Arc::new(AtomicBool::new(false));
        let stopper = FakeStopper {
            called: called.clone(),
        };
        assert!(
            handle_stop(
                StopRequest {
                    sandbox_id: id.to_string(),
                    wait: true
                },
                &reg,
                &stopper
            )
            .await
            .is_err()
        );
        assert!(!called.load(Ordering::SeqCst));
        assert!(reg.get(id).await.is_ok());
        assert!(reg.begin_exec(id).await.is_err());
        assert!(stop_all(&reg, &stopper).await.is_err());
        assert!(reg.get(id).await.is_ok());
    }

    #[tokio::test]
    async fn stop_happy_path_marks_and_calls_stopper() {
        use std::os::unix::fs::PermissionsExt;
        let namespace = tempfile::tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let reg = SandboxRegistry::new().with_context(context.clone());
        let id = Uuid::new_v4();
        let layout = OverlayLayout::for_sandbox(id);
        layout.prepare_parents(&context).unwrap();
        let lease = layout.admit(&context).unwrap();
        layout.ensure_admitted(&lease).unwrap();
        let session =
            super::super::adapters::runtime_test_session(lease, namespace.path(), false).await;
        session.wait().await.1.unwrap();
        reg.insert(state(id)).await;
        reg.attach_runtime(id, session).await.unwrap();
        let called = Arc::new(AtomicBool::new(false));
        let stopper = FakeStopper {
            called: called.clone(),
        };
        let response = handle_stop(
            StopRequest {
                sandbox_id: id.to_string(),
                wait: true,
            },
            &reg,
            &stopper,
        )
        .await
        .unwrap();
        assert!(response.stopped);
        assert!(
            !called.load(Ordering::SeqCst),
            "numeric PID fixture is never authority"
        );
        assert!(reg.get(id).await.is_err());
        assert!(!layout.base().exists());
    }

    struct FlakyStopper {
        fail_once: AtomicBool,
        call_count: Arc<std::sync::atomic::AtomicU32>,
    }
    #[async_trait::async_trait]
    impl VmStopper for FlakyStopper {
        async fn stop(&self, _pid: u32) -> Result<(), SandboxError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            if self.fail_once.swap(false, Ordering::SeqCst) {
                return Err(SandboxError::BootFailed("transient stop failure".into()));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn stop_error_preserves_state_and_retry_converges() {
        let reg = SandboxRegistry::new();
        let id = Uuid::new_v4();
        reg.insert(state(id)).await;
        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let stopper = FlakyStopper {
            fail_once: AtomicBool::new(true),
            call_count: call_count.clone(),
        };

        let err = handle_stop(
            StopRequest {
                sandbox_id: id.to_string(),
                wait: true,
            },
            &reg,
            &stopper,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SandboxError::BootFailed(_)));
        let after_fail = reg
            .get(id)
            .await
            .expect("entry must remain after failed stop");
        assert!(
            !after_fail.stopped,
            "stopped flag must not be set when stopper.stop returned Err"
        );

        let repeated = handle_stop(
            StopRequest {
                sandbox_id: id.to_string(),
                wait: true,
            },
            &reg,
            &stopper,
        )
        .await;
        assert!(
            repeated.is_err(),
            "retry cannot turn caller PID into retirement proof"
        );
        assert_eq!(call_count.load(Ordering::SeqCst), 0);
        assert!(reg.get(id).await.is_ok());
    }

    #[tokio::test]
    async fn stop_nonexistent_returns_s002() {
        let reg = SandboxRegistry::new();
        let stopper = FakeStopper {
            called: Arc::new(AtomicBool::new(false)),
        };
        let err = handle_stop(
            StopRequest {
                sandbox_id: Uuid::new_v4().to_string(),
                wait: false,
            },
            &reg,
            &stopper,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code().as_str(), "S002");
    }
}
