// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! `sandbox-builtin::worker::*`: worker VMs that Compose supervises.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::cli::local_worker::validate_source;
use crate::cli::worker_manager::oci;
use crate::sandbox_daemon::SandboxError;
use crate::wire::{WorkerPrepareRequest, WorkerPrepareResponse, WorkerSource};

/// The worker VMs of one project, behind its token and host roots.
pub struct WorkerVms {
    /// Per-VM state lives in `<base>/<vm_id>/`.
    #[expect(dead_code, reason = "worker::start reads it (Task 7)")]
    base: PathBuf,
    token: Option<String>,
    roots: Vec<PathBuf>,
}

impl WorkerVms {
    pub fn new(base: PathBuf, token: Option<String>, roots: Vec<PathBuf>) -> Self {
        Self { base, token, roots }
    }

    /// Compose sets the token and the roots on this container only.
    pub fn from_env() -> Self {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".iii/sandbox/workers");
        let token = std::env::var("III_SANDBOX_WORKER_TOKEN").ok();
        let roots = std::env::var_os("III_SANDBOX_HOST_ROOTS")
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Self::new(base, token, roots)
    }

    /// Validate the source and pull its base image (and, under overlay, build
    /// the shared base erofs) so `worker::start` stays fast. Idempotent.
    pub async fn prepare(
        &self,
        req: WorkerPrepareRequest,
    ) -> Result<WorkerPrepareResponse, SandboxError> {
        self.authorize(&req.auth)?;
        let (package, host_path, run_override) = match &req.source {
            WorkerSource::Bundle { package, host_path } => {
                (Some(package.as_str()), host_path, None)
            }
            WorkerSource::Local {
                host_path,
                run_override,
            } => (None, host_path, run_override.as_deref()),
        };
        let dir = self.admit(host_path)?;
        let (project, kind) =
            validate_source(&dir, package, run_override).map_err(SandboxError::SourceRejected)?;
        let rootfs = oci::prepare_rootfs(&kind, project.base_image.as_deref())
            .await
            .map_err(|e| SandboxError::PrepareFailed(format!("{e:#}")))?;
        if crate::cli::overlay::overlay_active() {
            tokio::task::spawn_blocking(move || crate::cli::erofs::ensure_base_erofs(&rootfs))
                .await
                .map_err(|e| SandboxError::PrepareFailed(e.to_string()))?
                .map_err(SandboxError::PrepareFailed)?;
        }
        Ok(WorkerPrepareResponse {})
    }

    /// Constant-time token check; no token (or an empty one) refuses every call.
    fn authorize(&self, auth: &str) -> Result<(), SandboxError> {
        let token = self.token.as_deref().unwrap_or("");
        let diff = token
            .bytes()
            .zip(auth.bytes())
            .fold(0, |d, (a, b)| d | (a ^ b));
        if token.is_empty() || token.len() != auth.len() || diff != 0 {
            return Err(SandboxError::Unauthorized);
        }
        Ok(())
    }

    /// `host_path`, canonicalized, if it lies under a canonicalized root.
    fn admit(&self, host_path: &str) -> Result<PathBuf, SandboxError> {
        if let Ok(path) = Path::new(host_path).canonicalize()
            && self
                .roots
                .iter()
                .filter_map(|root| root.canonicalize().ok())
                .any(|root| path.starts_with(root))
        {
            return Ok(path);
        }
        if self.roots.is_empty() {
            return Err(SandboxError::SourceRejected(format!(
                "no host roots are configured (III_SANDBOX_HOST_ROOTS is empty); \
                 refusing host_path {host_path}"
            )));
        }
        let roots: Vec<_> = self.roots.iter().map(|r| r.display().to_string()).collect();
        Err(SandboxError::SourceRejected(format!(
            "host_path {host_path} is outside the host roots [{}] (III_SANDBOX_HOST_ROOTS)",
            roots.join(", ")
        )))
    }
}

/// Stable per `state_key`: the first 32 hex chars of its SHA-256.
pub fn vm_id(state_key: &str) -> String {
    hex::encode(Sha256::digest(state_key))[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vms(t: &Path, token: Option<&str>, roots: Vec<PathBuf>) -> WorkerVms {
        WorkerVms::new(t.join("workers"), token.map(str::to_string), roots)
    }

    fn local(host_path: &Path) -> WorkerSource {
        WorkerSource::Local {
            host_path: host_path.display().to_string(),
            run_override: None,
        }
    }

    async fn prepare_err(vms: &WorkerVms, auth: &str, source: WorkerSource) -> SandboxError {
        vms.prepare(WorkerPrepareRequest {
            auth: auth.to_string(),
            source,
        })
        .await
        .expect_err("prepare must be refused")
    }

    #[tokio::test]
    async fn worker_ops_answer_s505_without_the_token() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        for token in [None, Some("t")] {
            let err = prepare_err(&vms(t, token, vec![t.into()]), "u", local(t)).await;
            assert_eq!(err.code().as_str(), "S505", "{err}");
        }
    }

    #[tokio::test]
    async fn host_paths_outside_the_roots_answer_s501() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        std::os::unix::fs::symlink("/etc", t.join("esc")).unwrap();
        let t_str = t.display().to_string();
        let cases = [
            (vec![t.to_path_buf()], PathBuf::from("/etc"), t_str.as_str()),
            (vec![t.to_path_buf()], t.join("../etc"), &t_str),
            (vec![t.to_path_buf()], t.join("esc"), &t_str),
            (vec![], t.to_path_buf(), "no host roots are configured"),
        ];
        for (roots, host_path, needle) in cases {
            let err = prepare_err(&vms(t, Some("t"), roots), "t", local(&host_path)).await;
            let msg = err.to_string();
            assert_eq!(err.code().as_str(), "S501", "{msg}");
            assert!(msg.contains("III_SANDBOX_HOST_ROOTS"), "{msg}");
            assert!(msg.contains(needle), "{msg}");
        }
    }

    #[test]
    fn host_roots_accept_a_path_under_a_symlinked_root() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        std::fs::create_dir_all(t.join("real/w")).unwrap();
        std::os::unix::fs::symlink(t.join("real"), t.join("link")).unwrap();
        let vms = vms(t, Some("t"), vec![t.join("link")]);
        for path in [t.join("real/w"), t.join("link/w")] {
            assert!(vms.admit(&path.display().to_string()).is_ok(), "{path:?}");
        }

        // macOS: /tmp is a symlink to /private/tmp.
        #[cfg(target_os = "macos")]
        {
            let tmp = tempfile::tempdir_in("/tmp").unwrap();
            std::fs::create_dir(tmp.path().join("w")).unwrap();
            let vms = WorkerVms::new(t.join("workers"), None, vec![tmp.path().into()]);
            let path = tmp.path().join("w").display().to_string();
            assert!(vms.admit(&path).is_ok(), "{path}");
        }
    }

    #[tokio::test]
    async fn a_bundle_naming_another_package_is_s501() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let bundle = t.join("bundle");
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(
            bundle.join("iii.worker.yaml"),
            "name: other\nscripts:\n  start: node bundle.js\n",
        )
        .unwrap();
        let source = WorkerSource::Bundle {
            package: "probe".to_string(),
            host_path: bundle.display().to_string(),
        };

        let err = prepare_err(&vms(t, Some("t"), vec![t.into()]), "t", source).await;
        assert_eq!(err.code().as_str(), "S501", "{err}");
        assert!(err.to_string().contains("\"probe\""), "{err}");
    }

    #[test]
    fn vm_id_is_stable_and_32_hex() {
        assert_eq!(vm_id("a"), vm_id("a"));
        assert_eq!(vm_id("a").len(), 32);
        assert!(vm_id("a").bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(vm_id("a"), vm_id("b"));
    }
}
