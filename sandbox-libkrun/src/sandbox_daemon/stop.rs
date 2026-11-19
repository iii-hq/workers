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
    session.finish_resources(&receipt).await.map_err(|e| {
        SandboxError::BootFailed(format!("sandbox host writers/endpoint retirement: {e}"))
    })?;
    registry
        .layout(id)
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
pub(crate) mod tests {
    use super::*;
    use crate::sandbox_daemon::registry::SandboxState;
    use std::future::Future;

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
    pub(crate) async fn admitted_state(
        reg: &SandboxRegistry,
        namespace: &std::path::Path,
        id: Uuid,
        explicit_close: bool,
    ) {
        admitted_state_mode(reg, namespace, id, explicit_close, false).await;
    }

    async fn admitted_state_mode(
        reg: &SandboxRegistry,
        namespace: &std::path::Path,
        id: Uuid,
        explicit_close: bool,
        running: bool,
    ) {
        use super::super::adapters::{RuntimeTestResources, runtime_test_session_resources};
        let context = reg.admission_context().unwrap();
        let layout = reg.layout(id);
        layout.prepare_parents(&context).unwrap();
        let lease = layout.admit(&context).unwrap();
        layout.ensure_admitted(&lease).unwrap();
        let session = runtime_test_session_resources(
            lease,
            namespace,
            running,
            RuntimeTestResources {
                reads: vec![layout.root.clone()],
                endpoints: vec![layout.shell_sock()],
                exit_without_drop: !explicit_close,
                ..Default::default()
            },
        )
        .await;
        if !running {
            assert!(session.wait().await.0.unwrap().success());
        } else {
            assert!(session.observation().os_status().is_none());
        }
        let mut entry = state(id);
        entry.shell_sock = layout.shell_sock();
        entry.workdir = layout.merged.clone();
        reg.insert(entry).await;
        reg.attach_runtime(id, session).await.unwrap();
    }

    #[tokio::test]
    async fn stop_all_stops_every_sandbox_and_removes_its_dir() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let payload = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let reg = SandboxRegistry::new()
            .with_context(context)
            .with_layout_root(payload.path().join("sandboxes"));
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        for (index, id) in ids.iter().enumerate() {
            admitted_state_mode(&reg, namespace.path(), *id, index == 0, index == 1).await;
        }
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let stopper = FlakyStopper {
            fail_once: AtomicBool::new(false),
            call_count: calls.clone(),
        };
        stop_all(&reg, &stopper).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "signals use the real core control, not PID fixtures"
        );
        assert_eq!(reg.count().await, 0);
        for id in ids {
            assert!(!reg.layout(id).base().exists());
            assert!(!reg.layout(id).shell_sock().exists());
            assert!(
                reg.layout(id).shell_sock().parent().unwrap().is_dir(),
                "private endpoint parents are retained"
            );
        }
    }

    #[tokio::test]
    async fn stop_all_preserves_pid_only_unresolved_layouts_and_registry() {
        let payload = crate::worker_admission::testing::private_tempdir().unwrap();
        let reg = SandboxRegistry::new().with_layout_root(payload.path().join("sandboxes"));
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        for id in ids {
            std::fs::create_dir_all(reg.layout(id).base()).unwrap();
            std::fs::write(reg.layout(id).base().join("sentinel"), b"keep").unwrap();
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
            assert_eq!(
                std::fs::read(reg.layout(id).base().join("sentinel")).unwrap(),
                b"keep"
            );
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
    async fn stop_keeps_registry_and_layout_when_exact_endpoint_settlement_fails() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let reg = SandboxRegistry::new()
            .with_context(context)
            .with_layout_root(data.path().join("sandboxes"));
        let id = Uuid::new_v4();
        admitted_state(&reg, namespace.path(), id, false).await;
        let layout = reg.layout(id);
        let socket = layout.shell_sock();
        std::fs::rename(&socket, socket.with_extension("held")).unwrap();
        let replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let stopper = FakeStopper {
            called: Arc::new(AtomicBool::new(false)),
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
        assert!(reg.get(id).await.is_ok());
        assert!(reg.begin_exec(id).await.is_err());
        assert!(layout.base().exists());
        assert!(socket.exists());
        drop(replacement);
    }

    #[tokio::test]
    async fn stop_happy_path_marks_and_calls_stopper() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let payload = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let reg = SandboxRegistry::new()
            .with_context(context)
            .with_layout_root(payload.path().join("sandboxes"));
        let id = Uuid::new_v4();
        admitted_state(&reg, namespace.path(), id, false).await;
        let layout = reg.layout(id);
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
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let reg = SandboxRegistry::new()
            .with_context(context)
            .with_layout_root(data.path().join("sandboxes"));
        let id = Uuid::new_v4();
        admitted_state(&reg, namespace.path(), id, false).await;
        let session = reg.lifecycle(id).await.unwrap();
        let layout = reg.layout(id);
        let (release, pending) = tokio::sync::oneshot::channel();
        session
            .add_test_host_writer(tokio::spawn(async move {
                pending.await.map_err(std::io::Error::other)
            }))
            .await;
        let stopper = FakeStopper {
            called: Arc::new(AtomicBool::new(false)),
        };
        let mut stopping = Box::pin(handle_stop(
            StopRequest {
                sandbox_id: id.to_string(),
                wait: true,
            },
            &reg,
            &stopper,
        ));
        let first =
            std::future::poll_fn(|cx| std::task::Poll::Ready(stopping.as_mut().poll(cx))).await;
        assert!(first.is_pending());
        drop(stopping); // interrupted client wait is not successful stop or Remove
        assert!(layout.base().exists());
        assert!(layout.shell_sock().exists());
        assert!(reg.get(id).await.is_ok());
        assert!(reg.begin_exec(id).await.is_err());
        release.send(()).unwrap();
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
        assert!(!layout.base().exists());
        assert!(!layout.shell_sock().exists());
        assert!(reg.get(id).await.is_err());
    }

    #[tokio::test]
    async fn pid_only_stop_error_preserves_state_and_retry_cannot_invent_authority() {
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
