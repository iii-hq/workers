use std::path::Path;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{register_fn, run_text, spec, EmptyInput, FunctionSpec};
use crate::config::{SharedConfig, WorkerConfig};

pub const FILE_TARGETS_ID: &str = "tailscale::file::targets";
pub const FILE_SEND_ID: &str = "tailscale::file::send";
pub const FILE_RECEIVE_ID: &str = "tailscale::file::receive";
pub const CERT_ID: &str = "tailscale::cert";
pub const DRIVE_LIST_ID: &str = "tailscale::drive::list";
pub const DRIVE_SHARE_ID: &str = "tailscale::drive::share";
pub const DRIVE_UNSHARE_ID: &str = "tailscale::drive::unshare";

const FILE_TARGETS_DESC: &str =
    "List the tailnet devices that accept Taildrop files from this node.";
const FILE_SEND_DESC: &str = "Send files to a tailnet device with Taildrop (`tailscale file cp`). Paths must be absolute and exist on this host.";
const FILE_RECEIVE_DESC: &str = "Move files that arrived in this node's Taildrop inbox into a directory (`tailscale file get`).";
const CERT_DESC: &str = "Fetch a Let's Encrypt certificate and key for one of this node's MagicDNS domains (`tailscale cert`). Requires HTTPS enabled for the tailnet.";
const DRIVE_LIST_DESC: &str = "List the directories this node shares with the tailnet through Taildrive. The macOS GUI app manages Taildrive in its own settings and rejects the CLI.";
const DRIVE_SHARE_DESC: &str =
    "Share a directory with the tailnet through Taildrive under a name (`tailscale drive share`).";
const DRIVE_UNSHARE_DESC: &str =
    "Stop sharing a Taildrive directory by name (`tailscale drive unshare`).";

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<EmptyInput, FileTargetsOutput>(FILE_TARGETS_ID, FILE_TARGETS_DESC),
        spec::<FileSendInput, CommandOutput>(FILE_SEND_ID, FILE_SEND_DESC),
        spec::<FileReceiveInput, CommandOutput>(FILE_RECEIVE_ID, FILE_RECEIVE_DESC),
        spec::<CertInput, CertOutput>(CERT_ID, CERT_DESC),
        spec::<EmptyInput, DriveListOutput>(DRIVE_LIST_ID, DRIVE_LIST_DESC),
        spec::<DriveShareInput, DriveListOutput>(DRIVE_SHARE_ID, DRIVE_SHARE_DESC),
        spec::<DriveUnshareInput, DriveListOutput>(DRIVE_UNSHARE_ID, DRIVE_UNSHARE_DESC),
    ]
}

pub fn register(iii: &IIIClient, config: &SharedConfig) {
    register_fn!(
        iii,
        config,
        FILE_TARGETS_ID,
        FILE_TARGETS_DESC,
        EmptyInput,
        file_targets
    );
    register_fn!(
        iii,
        config,
        FILE_SEND_ID,
        FILE_SEND_DESC,
        FileSendInput,
        file_send
    );
    register_fn!(
        iii,
        config,
        FILE_RECEIVE_ID,
        FILE_RECEIVE_DESC,
        FileReceiveInput,
        file_receive
    );
    register_fn!(iii, config, CERT_ID, CERT_DESC, CertInput, cert);
    register_fn!(
        iii,
        config,
        DRIVE_LIST_ID,
        DRIVE_LIST_DESC,
        EmptyInput,
        drive_list
    );
    register_fn!(
        iii,
        config,
        DRIVE_SHARE_ID,
        DRIVE_SHARE_DESC,
        DriveShareInput,
        drive_share
    );
    register_fn!(
        iii,
        config,
        DRIVE_UNSHARE_ID,
        DRIVE_UNSHARE_DESC,
        DriveUnshareInput,
        drive_unshare
    );
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FileTarget {
    /// Tailscale IP of the device.
    pub ip: String,
    /// Machine name of the device.
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FileTargetsOutput {
    /// Devices that accept Taildrop files from this node.
    pub targets: Vec<FileTarget>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileSendInput {
    /// Absolute paths of the files to send.
    #[schemars(length(min = 1))]
    pub paths: Vec<String>,
    /// Receiving device by machine name or Tailscale IP.
    pub target: String,
    /// Alternate file name to use when sending a single file.
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileReceiveInput {
    /// Absolute directory that receives the inbox files.
    pub directory: String,
    /// What to do when a same-named file exists; defaults to `skip`.
    #[serde(default)]
    pub conflict: Conflict,
    /// Wait for at least one file to arrive before returning.
    #[serde(default)]
    pub wait: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    #[default]
    Skip,
    Overwrite,
    Rename,
}

impl Conflict {
    fn as_str(self) -> &'static str {
        match self {
            Conflict::Skip => "skip",
            Conflict::Overwrite => "overwrite",
            Conflict::Rename => "rename",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CommandOutput {
    /// True when the CLI exited successfully.
    pub ok: bool,
    /// The CLI's own output.
    pub output: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CertInput {
    /// One of this node's certificate domains, as reported by dns::status `cert_domains`.
    pub domain: String,
    /// Absolute path to write the certificate to.
    pub cert_file: String,
    /// Absolute path to write the private key to.
    pub key_file: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CertOutput {
    /// Domain the certificate was issued for.
    pub domain: String,
    /// Path of the written certificate.
    pub cert_file: String,
    /// Path of the written private key.
    pub key_file: String,
    /// The CLI's own output.
    pub output: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DriveListOutput {
    /// Taildrive shares as `name  path` lines from the CLI.
    pub shares: Vec<DriveShare>,
    /// The CLI's own output.
    pub output: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DriveShare {
    /// Share name.
    pub name: String,
    /// Directory shared.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DriveShareInput {
    /// Share name.
    pub name: String,
    /// Absolute directory to share.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DriveUnshareInput {
    /// Share name.
    pub name: String,
}

fn validate_absolute(path: &str, what: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() || !Path::new(path).is_absolute() || path.starts_with('-') {
        return Err(format!("{what} must be an absolute path"));
    }
    Ok(())
}

fn validate_name(value: &str, what: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_whitespace) {
        return Err(format!("{what} must not be empty or contain whitespace"));
    }
    Ok(value.to_string())
}

pub fn parse_targets(text: &str) -> Vec<FileTarget> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let ip = parts.next()?;
            let name = parts.next()?;
            Some(FileTarget {
                ip: ip.to_string(),
                name: name.to_string(),
            })
        })
        .collect()
}

async fn file_targets(config: &WorkerConfig, _: EmptyInput) -> Result<FileTargetsOutput, String> {
    let text = run_text(config, &["file", "cp", "--targets"]).await?;
    Ok(FileTargetsOutput {
        targets: parse_targets(&text),
    })
}

async fn file_send(config: &WorkerConfig, input: FileSendInput) -> Result<CommandOutput, String> {
    if input.paths.is_empty() {
        return Err("paths must contain at least one file".to_string());
    }
    for path in &input.paths {
        validate_absolute(path, "each path")?;
        if !Path::new(path.trim()).is_file() {
            return Err(format!("{path} is not a file on this host"));
        }
    }
    let target = validate_name(&input.target, "target")?;
    let mut args = vec!["file".to_string(), "cp".to_string()];
    if let Some(name) = &input.name {
        if input.paths.len() != 1 {
            return Err("name applies to a single file only".to_string());
        }
        args.push("--name".to_string());
        args.push(validate_name(name, "name")?);
    }
    args.extend(input.paths.iter().map(|p| p.trim().to_string()));
    args.push(format!("{target}:"));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = run_text(config, &refs).await?;
    Ok(CommandOutput { ok: true, output })
}

async fn file_receive(
    config: &WorkerConfig,
    input: FileReceiveInput,
) -> Result<CommandOutput, String> {
    validate_absolute(&input.directory, "directory")?;
    if !Path::new(input.directory.trim()).is_dir() {
        return Err("directory must exist on this host".to_string());
    }
    let conflict_arg = format!("--conflict={}", input.conflict.as_str());
    let mut args = vec!["file", "get", &conflict_arg];
    if input.wait {
        args.push("--wait");
    }
    let directory = input.directory.trim().to_string();
    args.push(&directory);
    let output = run_text(config, &args).await?;
    Ok(CommandOutput { ok: true, output })
}

async fn cert(config: &WorkerConfig, input: CertInput) -> Result<CertOutput, String> {
    let domain = validate_name(&input.domain, "domain")?;
    validate_absolute(&input.cert_file, "cert_file")?;
    validate_absolute(&input.key_file, "key_file")?;
    let cert_file = input.cert_file.trim().to_string();
    let key_file = input.key_file.trim().to_string();
    let output = run_text(
        config,
        &[
            "cert",
            "--cert-file",
            &cert_file,
            "--key-file",
            &key_file,
            &domain,
        ],
    )
    .await?;
    Ok(CertOutput {
        domain,
        cert_file,
        key_file,
        output,
    })
}

pub fn parse_drive_list(text: &str) -> Vec<DriveShare> {
    text.lines()
        .filter_map(|line| {
            let (name, path) = line.trim().split_once(char::is_whitespace)?;
            Some(DriveShare {
                name: name.to_string(),
                path: path.trim().to_string(),
            })
        })
        .filter(|share| share.path.starts_with('/'))
        .collect()
}

async fn drive_list(config: &WorkerConfig, _: EmptyInput) -> Result<DriveListOutput, String> {
    let output = run_text(config, &["drive", "list"]).await?;
    Ok(DriveListOutput {
        shares: parse_drive_list(&output),
        output,
    })
}

async fn drive_share(
    config: &WorkerConfig,
    input: DriveShareInput,
) -> Result<DriveListOutput, String> {
    let name = validate_name(&input.name, "name")?;
    validate_absolute(&input.path, "path")?;
    let path = input.path.trim().to_string();
    run_text(config, &["drive", "share", &name, &path]).await?;
    drive_list(config, EmptyInput::default()).await
}

async fn drive_unshare(
    config: &WorkerConfig,
    input: DriveUnshareInput,
) -> Result<DriveListOutput, String> {
    let name = validate_name(&input.name, "name")?;
    run_text(config, &["drive", "unshare", &name]).await?;
    drive_list(config, EmptyInput::default()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taildrop_targets_are_parsed() {
        let targets = parse_targets("100.64.0.2\tphone\n100.64.0.3   laptop\n\n");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].ip, "100.64.0.2");
        assert_eq!(targets[1].name, "laptop");
    }

    #[test]
    fn drive_shares_are_parsed() {
        let shares = parse_drive_list(
            "docs    /Users/me/Documents\nphotos  /Users/me/Pictures\nnot a share\n",
        );
        assert_eq!(shares.len(), 2);
        assert_eq!(shares[0].name, "docs");
        assert_eq!(shares[1].path, "/Users/me/Pictures");
    }

    #[test]
    fn absolute_paths_and_names_are_enforced() {
        assert!(validate_absolute("/tmp/a", "path").is_ok());
        assert!(validate_absolute("tmp/a", "path").is_err());
        assert!(validate_name("phone", "target").is_ok());
        assert!(validate_name("--targets", "target").is_err());
        assert!(validate_name("two words", "target").is_err());
    }
}
