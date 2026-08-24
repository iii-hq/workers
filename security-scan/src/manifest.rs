//! Side-effect-free worker metadata for the registry publish pipeline.

use serde::Serialize;

pub const DESCRIPTION: &str = "Report-only security reviews of operator-configured repositories at immutable Git commits, dispatched through a read-only Harness policy.";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: DESCRIPTION.to_string(),
        default_config: serde_json::json!({
            "repositories": [],
            "analysis": {
                "model": "",
                "max_turns": 4,
                "max_output_tokens": 8_000,
                "max_total_tokens": 50_000,
                "max_cost_usd": 2.0,
            },
        }),
        supported_targets: vec![build_target()],
    }
}

fn build_target() -> String {
    if let Some(target) = option_env!("TARGET") {
        return target.to_string();
    }

    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin".to_string()
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin".to_string()
    } else if cfg!(all(
        target_os = "windows",
        target_env = "msvc",
        target_arch = "aarch64"
    )) {
        "aarch64-pc-windows-msvc".to_string()
    } else if cfg!(all(
        target_os = "windows",
        target_env = "msvc",
        target_arch = "x86_64"
    )) {
        "x86_64-pc-windows-msvc".to_string()
    } else if cfg!(all(
        target_os = "windows",
        target_env = "msvc",
        target_arch = "x86"
    )) {
        "i686-pc-windows-msvc".to_string()
    } else if cfg!(all(
        target_os = "linux",
        target_env = "musl",
        target_arch = "x86_64"
    )) {
        "x86_64-unknown-linux-musl".to_string()
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "aarch64"
    )) {
        "aarch64-unknown-linux-gnu".to_string()
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "x86_64"
    )) {
        "x86_64-unknown-linux-gnu".to_string()
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "arm"
    )) {
        "armv7-unknown-linux-gnueabihf".to_string()
    } else {
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    }
}
