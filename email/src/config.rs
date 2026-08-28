use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountConfig>,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    pub provider: Provider,
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp: Option<SmtpConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap: Option<ImapConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Smtp,
    Imap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_true")]
    pub starttls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_true")]
    pub tls: bool,
    #[serde(default = "default_folders")]
    pub folders: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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

fn configured_credential(username: &Option<String>, password: &Option<String>) -> Option<Value> {
    match (username, password) {
        (Some(user), Some(pass)) if !user.is_empty() && !pass.is_empty() => Some(json!({
            "type": "api_key",
            "username": user,
            "password": pass,
        })),
        _ => None,
    }
}

impl SmtpConfig {
    pub fn credential(&self) -> Option<Value> {
        configured_credential(&self.username, &self.password)
    }
}

impl ImapConfig {
    pub fn credential(&self) -> Option<Value> {
        configured_credential(&self.username, &self.password)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BootSignature {
    pub accounts: BTreeMap<String, AccountConfig>,
}

impl WorkerConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), WorkerConfig::default().to_json());
        }
        schema
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, account) in &self.accounts {
            if name.trim().is_empty() {
                return Err("account names must not be empty".to_string());
            }
            match account.provider {
                Provider::Smtp if account.smtp.is_none() => {
                    return Err(format!(
                        "account `{name}`: provider smtp needs an `smtp` block"
                    ));
                }
                Provider::Imap if account.imap.is_none() => {
                    return Err(format!(
                        "account `{name}`: provider imap needs an `imap` block"
                    ));
                }
                _ => {}
            }
        }
        if self.limits.max_recipients == 0 {
            return Err("limits.max_recipients must be at least 1".to_string());
        }
        Ok(())
    }

    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            accounts: self.accounts.clone(),
        }
    }
}

fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        if name.contains(':') {
            out.push_str("${");
            out.push_str(&after[..=end]);
        } else {
            match std::env::var(name) {
                Ok(val) => out.push_str(&val),
                Err(_) => tracing::warn!(
                    var = %name,
                    "seed config references unset env var; expanding to empty string"
                ),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMTP_ACCOUNT: &str = r#"
accounts:
  notifications:
    provider: smtp
    from: "ReachAI <noreply@example.com>"
    smtp:
      host: smtp.resend.com
      port: 587
      username: resend
      password: ${EMAIL_TEST_SMTP_PASSWORD}
"#;

    #[test]
    fn seed_yaml_expands_env_into_credentials() {
        std::env::set_var("EMAIL_TEST_SMTP_PASSWORD", "re_secret");
        let cfg = WorkerConfig::from_yaml(SMTP_ACCOUNT).unwrap();
        let smtp = cfg.accounts["notifications"].smtp.as_ref().unwrap();
        assert!(smtp.starttls);
        let cred = smtp.credential().unwrap();
        assert_eq!(cred["username"], "resend");
        assert_eq!(cred["password"], "re_secret");
        assert_eq!(cred["type"], "api_key");
    }

    #[test]
    fn worker_side_placeholders_are_left_to_the_configuration_worker() {
        assert_eq!(expand_env("${HOST:localhost}"), "${HOST:localhost}");
        assert_eq!(expand_env("${"), "${");
    }

    #[test]
    fn credential_requires_both_halves() {
        let smtp = SmtpConfig {
            host: "h".into(),
            port: 25,
            starttls: true,
            username: Some("u".into()),
            password: None,
        };
        assert!(smtp.credential().is_none());
        let imap = ImapConfig {
            host: "h".into(),
            port: 993,
            tls: true,
            folders: default_folders(),
            username: Some("u".into()),
            password: Some("p".into()),
        };
        assert_eq!(imap.credential().unwrap()["password"], "p");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = WorkerConfig::from_yaml("accounts: {}\nlimit: {}\n").unwrap_err();
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn validate_requires_the_transport_block_for_the_provider() {
        let cfg =
            WorkerConfig::from_yaml("accounts:\n  inbox:\n    provider: imap\n    from: a@b.c\n")
                .unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("needs an `imap` block"), "{err}");
        assert!(WorkerConfig::default().validate().is_ok());
    }

    #[test]
    fn boot_signature_ignores_limits() {
        let base = WorkerConfig::from_yaml(SMTP_ACCOUNT).unwrap();
        let mut tuned = base.clone();
        tuned.limits.max_recipients = 5;
        assert_eq!(base.boot_signature(), tuned.boot_signature());
        let mut moved = base.clone();
        moved.accounts.get_mut("notifications").unwrap().from = "Other <o@example.com>".into();
        assert_ne!(base.boot_signature(), moved.boot_signature());
    }

    #[test]
    fn json_round_trip_and_schema_example() {
        let cfg = WorkerConfig::from_yaml(SMTP_ACCOUNT).unwrap();
        let back = WorkerConfig::from_json(&cfg.to_json()).unwrap();
        assert_eq!(cfg, back);
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["example"], WorkerConfig::default().to_json());
        assert!(schema["properties"]["accounts"].is_object());
    }
}
