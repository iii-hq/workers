use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkerConfig {
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountConfig>,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountConfig {
    pub provider: Provider,
    pub from: String,
    #[serde(default)]
    pub smtp: Option<SmtpConfig>,
    #[serde(default)]
    pub imap: Option<ImapConfig>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Smtp,
    Imap,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_true")]
    pub starttls: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_true")]
    pub tls: bool,
    #[serde(default = "default_folders")]
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Limits {
    #[serde(default = "default_max_attach")]
    pub max_attachment_bytes: usize,
    #[serde(default = "default_max_recipients")]
    pub max_recipients: usize,
    #[serde(default = "default_send_timeout")]
    pub send_timeout_ms: u64,
    #[serde(default = "default_imap_connect_timeout")]
    pub imap_connect_timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_attachment_bytes: default_max_attach(),
            max_recipients: default_max_recipients(),
            send_timeout_ms: default_send_timeout(),
            imap_connect_timeout_ms: default_imap_connect_timeout(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_folders() -> Vec<String> {
    vec!["INBOX".to_string()]
}
fn default_max_attach() -> usize {
    26_214_400
}
fn default_max_recipients() -> usize {
    100
}
fn default_send_timeout() -> u64 {
    30_000
}
fn default_imap_connect_timeout() -> u64 {
    15_000
}

impl WorkerConfig {
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref())?;
        let cfg: Self = serde_yaml::from_str(&raw)?;
        Ok(cfg)
    }
}
