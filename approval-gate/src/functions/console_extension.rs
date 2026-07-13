//! Runtime-owned console extension capability.
//!
//! The UI is part of the approval-gate binary, not the console build. The
//! console discovers `approval::console-extension`, fetches these allowlisted
//! assets over the existing engine bus, verifies their etags, and activates
//! the ES module. Both functions are registered as internal control-plane
//! handlers so agents never see or invoke them.

use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::ApprovalError;

const EXTENSION_JS: &[u8] = include_bytes!("../../web/dist/extension.js");
const EXTENSION_CSS: &[u8] = include_bytes!("../../web/dist/extension.css");

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
/// The engine may add `_caller_worker_id` to control-plane requests in transit.
pub struct ConsoleExtensionManifestRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ConsoleExtensionAssetDescriptor {
    pub path: String,
    pub media_type: String,
    /// Stable content identifier used for verification and browser caching.
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ConsoleExtensionManifestResponse {
    pub id: String,
    pub api_version: u32,
    pub worker_version: String,
    pub asset_function: String,
    pub entry: ConsoleExtensionAssetDescriptor,
    pub styles: Vec<ConsoleExtensionAssetDescriptor>,
    pub slots: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
/// The engine may add `_caller_worker_id` to control-plane requests in transit.
pub struct ConsoleExtensionAssetRequest {
    /// Allowlisted path from the capability manifest.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ConsoleExtensionAssetResponse {
    pub path: String,
    pub media_type: String,
    pub encoding: String,
    pub content: String,
    pub etag: String,
}

fn content_etag(bytes: &[u8]) -> String {
    // FNV-1a is intentionally a cache/content identity, not a signature. The
    // native worker binary is already the trust boundary; this detects stale
    // or corrupted transport without adding another crypto dependency.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64-{hash:016x}")
}

fn descriptor(path: &str, media_type: &str, bytes: &[u8]) -> ConsoleExtensionAssetDescriptor {
    ConsoleExtensionAssetDescriptor {
        path: path.to_string(),
        media_type: media_type.to_string(),
        etag: content_etag(bytes),
    }
}

pub async fn manifest(
    _deps: &Deps,
    _req: ConsoleExtensionManifestRequest,
) -> Result<ConsoleExtensionManifestResponse, ApprovalError> {
    Ok(ConsoleExtensionManifestResponse {
        id: "approval-gate".to_string(),
        api_version: 1,
        worker_version: env!("CARGO_PKG_VERSION").to_string(),
        asset_function: "approval::console-extension::asset".to_string(),
        entry: descriptor("extension.js", "text/javascript", EXTENSION_JS),
        styles: vec![descriptor("extension.css", "text/css", EXTENSION_CSS)],
        slots: vec![
            "chat.banner".to_string(),
            "chat.composer.controls".to_string(),
            "function-call.pending-actions".to_string(),
            "settings.sections".to_string(),
            "chat.workspace-access".to_string(),
        ],
    })
}

pub async fn asset(
    _deps: &Deps,
    req: ConsoleExtensionAssetRequest,
) -> Result<ConsoleExtensionAssetResponse, ApprovalError> {
    let (media_type, bytes) = match req.path.as_str() {
        "extension.js" => ("text/javascript", EXTENSION_JS),
        "extension.css" => ("text/css", EXTENSION_CSS),
        _ => {
            return Err(ApprovalError::InvalidPayload(format!(
                "unknown console extension asset `{}`",
                req.path
            )))
        }
    };
    Ok(ConsoleExtensionAssetResponse {
        path: req.path,
        media_type: media_type.to_string(),
        encoding: "base64".to_string(),
        content: base64::engine::general_purpose::STANDARD.encode(bytes),
        etag: content_etag(bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{with_stack, BootOpts};

    #[test]
    fn content_etag_matches_the_console_contract() {
        assert_eq!(content_etag(b"hello"), "fnv1a64-a430d84680aabd0b");
    }

    #[test]
    fn requests_accept_engine_caller_metadata() {
        let manifest: ConsoleExtensionManifestRequest = serde_json::from_value(serde_json::json!({
            "_caller_worker_id": "console"
        }))
        .unwrap();
        let _ = manifest;

        let asset: ConsoleExtensionAssetRequest = serde_json::from_value(serde_json::json!({
            "path": "extension.js",
            "_caller_worker_id": "console"
        }))
        .unwrap();
        assert_eq!(asset.path, "extension.js");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn manifest_and_assets_are_consistent() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let manifest = manifest(&stack.deps, ConsoleExtensionManifestRequest {})
                .await
                .unwrap();
            assert_eq!(manifest.id, "approval-gate");
            assert_eq!(manifest.api_version, 1);
            assert!(manifest.slots.contains(&"chat.composer.controls".into()));

            for descriptor in std::iter::once(&manifest.entry).chain(manifest.styles.iter()) {
                let response = asset(
                    &stack.deps,
                    ConsoleExtensionAssetRequest {
                        path: descriptor.path.clone(),
                    },
                )
                .await
                .unwrap();
                assert_eq!(response.etag, descriptor.etag);
                assert_eq!(response.media_type, descriptor.media_type);
                assert!(!response.content.is_empty());
            }
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_paths_outside_the_allowlist() {
        with_stack(BootOpts::needs_approval(), |stack| async move {
            let err = asset(
                &stack.deps,
                ConsoleExtensionAssetRequest {
                    path: "../secrets".into(),
                },
            )
            .await
            .unwrap_err();
            assert_eq!(err.code(), "approval/invalid_payload");
        })
        .await;
    }
}
