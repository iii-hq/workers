//! `xcrun simctl`, scoped to one device set. Every call passes `--set` for a
//! tenant's private set, so a device in another tenant's set does not exist
//! as far as the call is concerned — isolation comes from CoreSimulator, not
//! from a check this worker could forget.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;

/// simctl calls that do not finish in this long are killed. Boot, erase and
/// app launch are the slow ones; they finish well inside it on a healthy host.
const TIMEOUT: Duration = Duration::from_secs(180);

/// One device set: `None` is the Mac's own set (`~/Library/Developer/CoreSimulator/Devices`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSet {
    pub path: Option<PathBuf>,
    pub developer_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Device {
    pub udid: String,
    pub name: String,
    /// `Booted`, `Shutdown`, `Booting`, `Shutting Down`, `Creating`.
    pub state: String,
    /// Runtime identifier, e.g. `com.apple.CoreSimulator.SimRuntime.iOS-26-5`.
    pub runtime: String,
    /// Device type identifier, e.g. `com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro`.
    pub device_type: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DeviceType {
    pub identifier: String,
    pub name: String,
    /// `iPhone`, `iPad`, `Apple Watch`, `Apple TV`, `Apple Vision`.
    pub product_family: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Runtime {
    pub identifier: String,
    /// e.g. `iOS 26.5`.
    pub name: String,
    pub version: String,
    /// Device types this runtime can run.
    pub device_types: Vec<DeviceType>,
}

/// `xcode-select -p`, the developer dir CoreSimulator and SimulatorKit live under.
pub async fn default_developer_dir() -> Result<PathBuf, String> {
    let out = Command::new("xcode-select")
        .arg("-p")
        .output()
        .await
        .map_err(|e| format!("xcode-select: {e}; install Xcode"))?;
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || dir.is_empty() {
        return Err("no Xcode developer directory; install Xcode and run xcode-select".into());
    }
    Ok(PathBuf::from(dir))
}

impl DeviceSet {
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("xcrun");
        cmd.env("DEVELOPER_DIR", &self.developer_dir).arg("simctl");
        if let Some(path) = &self.path {
            cmd.arg("--set").arg(path);
        }
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }

    /// A long-running simctl child (recordVideo) the caller manages.
    pub fn spawn(&self, args: &[&str]) -> Result<tokio::process::Child, String> {
        self.command(args)
            .spawn()
            .map_err(|e| format!("xcrun simctl: {e}"))
    }

    /// Run simctl to completion; stdout on success, stderr as the error.
    pub async fn run(&self, args: &[&str]) -> Result<String, String> {
        let out = tokio::time::timeout(TIMEOUT, self.command(args).output())
            .await
            .map_err(|_| format!("simctl {} timed out", args.first().unwrap_or(&"")))?
            .map_err(|e| format!("xcrun simctl: {e}; install Xcode"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err
                .trim()
                .lines()
                .last()
                .unwrap_or("simctl failed")
                .to_string())
        }
    }

    pub async fn devices(&self) -> Result<Vec<Device>, String> {
        // A tenant that never created a simulator has no set yet.
        if self.path.as_ref().is_some_and(|p| !p.is_dir()) {
            return Ok(Vec::new());
        }
        parse_devices(&self.run(&["list", "devices", "-j"]).await?)
    }

    pub async fn device(&self, udid: &str) -> Result<Device, String> {
        self.devices()
            .await?
            .into_iter()
            .find(|d| d.udid == udid)
            .ok_or_else(|| format!("unknown device '{udid}'"))
    }

    pub async fn runtimes(&self) -> Result<Vec<Runtime>, String> {
        parse_runtimes(&self.run(&["list", "runtimes", "-j"]).await?)
    }
}

pub fn parse_devices(json: &str) -> Result<Vec<Device>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("simctl list: {e}"))?;
    let mut out = Vec::new();
    for (runtime, devices) in v["devices"].as_object().into_iter().flatten() {
        for d in devices.as_array().into_iter().flatten() {
            out.push(Device {
                udid: d["udid"].as_str().unwrap_or_default().to_string(),
                name: d["name"].as_str().unwrap_or_default().to_string(),
                state: d["state"].as_str().unwrap_or_default().to_string(),
                runtime: runtime.clone(),
                device_type: d["deviceTypeIdentifier"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                available: d["isAvailable"].as_bool().unwrap_or(false),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.udid.cmp(&b.udid)));
    Ok(out)
}

pub fn parse_runtimes(json: &str) -> Result<Vec<Runtime>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("simctl list: {e}"))?;
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
    Ok(v["runtimes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["isAvailable"].as_bool().unwrap_or(false))
        .map(|r| Runtime {
            identifier: text(r, "identifier"),
            name: text(r, "name"),
            version: text(r, "version"),
            device_types: r["supportedDeviceTypes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| DeviceType {
                    identifier: text(t, "identifier"),
                    name: text(t, "name"),
                    product_family: text(t, "productFamily"),
                })
                .collect(),
        })
        .collect())
}

/// `true` for an RFC 4122 UUID in simctl's upper-case form. Everything that
/// reaches a filesystem path or a simctl argument is checked with this first.
pub fn is_udid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('A'..='F').contains(&c),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simctl_device_list() {
        let json = r#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-5":[
            {"udid":"5FEFD8EB-AB2D-4B6A-A5AB-6604312D1993","name":"iPhone 17 Pro","state":"Booted",
             "isAvailable":true,"deviceTypeIdentifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro"}]}}"#;
        let devices = parse_devices(json).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].state, "Booted");
        assert_eq!(
            devices[0].runtime,
            "com.apple.CoreSimulator.SimRuntime.iOS-26-5"
        );
    }

    #[test]
    fn parses_only_available_runtimes() {
        let json = r#"{"runtimes":[
            {"identifier":"a","name":"iOS 26.5","version":"26.5","isAvailable":true,
             "supportedDeviceTypes":[{"identifier":"t","name":"iPhone 17","productFamily":"iPhone"}]},
            {"identifier":"b","name":"iOS 18.0","version":"18.0","isAvailable":false}]}"#;
        let runtimes = parse_runtimes(json).unwrap();
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].device_types[0].product_family, "iPhone");
    }

    #[test]
    fn udid_check_rejects_paths_and_lowercase() {
        assert!(is_udid("5FEFD8EB-AB2D-4B6A-A5AB-6604312D1993"));
        assert!(!is_udid("5fefd8eb-ab2d-4b6a-a5ab-6604312d1993"));
        assert!(!is_udid("../../etc/passwd"));
        assert!(!is_udid("booted"));
    }
}
