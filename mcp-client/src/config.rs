use crate::refs::{Ref, Text};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const ROOT_KEY: &str = "mcpServers";

pub const KNOWN_FIELDS: [&str; 9] = [
    "type", "url", "headers", "command", "args", "env", "cwd", "auth", "disabled",
];

/// The `mcpServers` map as stored; a wrong entry stays as its raw value.
#[derive(Debug, Clone, PartialEq)]
pub struct RawConfig(pub BTreeMap<String, Value>);

impl RawConfig {
    pub fn from_value(value: &Value) -> RawConfig {
        let map = value
            .get(ROOT_KEY)
            .and_then(Value::as_object)
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        RawConfig(map)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub disabled: bool,
    pub transport: Transport,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    Http(HttpEntry),
    Stdio(StdioEntry),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpEntry {
    pub url: Text,
    pub headers: BTreeMap<String, Text>,
    pub auth: Auth,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StdioEntry {
    pub command: String,
    pub args: Vec<Text>,
    pub env: BTreeMap<String, Text>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Auth {
    None,
    Bearer {
        token: Ref,
    },
    ClientCredentials {
        token_url: String,
        client_id: String,
        client_secret: Ref,
        scope: Option<String>,
    },
    OAuth {
        scopes: Vec<String>,
        client_id: Option<String>,
        client_secret: Option<Ref>,
        client_metadata_url: Option<String>,
    },
}

fn text_str(t: &Text) -> String {
    match t {
        Text::Literal(s) => s.clone(),
        Text::Ref(r) => r.to_string(),
    }
}

impl Entry {
    /// Every reference once: http gives the URL, the header values, then the
    /// auth reference; stdio gives the args, then the env values.
    pub fn refs(&self) -> Vec<Ref> {
        let mut out: Vec<Ref> = Vec::new();
        let mut add = |r: &Ref| {
            if !out.contains(r) {
                out.push(r.clone());
            }
        };
        let mut add_text = |t: &Text| {
            if let Text::Ref(r) = t {
                add(r);
            }
        };
        match &self.transport {
            Transport::Http(h) => {
                add_text(&h.url);
                h.headers.values().for_each(&mut add_text);
                match &h.auth {
                    Auth::Bearer { token } => add_text(&Text::Ref(token.clone())),
                    Auth::ClientCredentials { client_secret, .. } => {
                        add_text(&Text::Ref(client_secret.clone()))
                    }
                    Auth::OAuth {
                        client_secret: Some(s),
                        ..
                    } => add_text(&Text::Ref(s.clone())),
                    _ => {}
                }
            }
            Transport::Stdio(s) => {
                s.args.iter().for_each(&mut add_text);
                s.env.values().for_each(&mut add_text);
            }
        }
        out
    }

    /// The URL or the command line, references as written.
    pub fn target(&self) -> String {
        match &self.transport {
            Transport::Http(h) => text_str(&h.url),
            Transport::Stdio(s) => std::iter::once(s.command.clone())
                .chain(s.args.iter().map(text_str))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    pub fn auth_kind(&self) -> &'static str {
        let Transport::Http(h) = &self.transport else {
            return "none";
        };
        match &h.auth {
            Auth::Bearer { .. } => "bearer",
            Auth::ClientCredentials { .. } => "client_credentials",
            Auth::OAuth { .. } => "oauth",
            Auth::None if h.headers.values().any(|v| matches!(v, Text::Ref(_))) => "header",
            Auth::None => "none",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refs::{Ref, Text};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn r(s: &str) -> Ref {
        Ref::parse(s).unwrap()
    }

    fn http(url: &str, headers: &[(&str, &str)], auth: Auth) -> Entry {
        Entry {
            name: "s".into(),
            disabled: false,
            transport: Transport::Http(HttpEntry {
                url: Text::parse(url),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), Text::parse(v)))
                    .collect(),
                auth,
            }),
        }
    }

    #[test]
    fn raw_config_never_panics_on_a_wrong_shape() {
        for root in [
            json!(null),
            json!("x"),
            json!([1]),
            json!({}),
            json!({"mcpServers": []}),
            json!({"mcpServers": "x"}),
        ] {
            assert!(RawConfig::from_value(&root).0.is_empty(), "{root}");
        }
        let raw = RawConfig::from_value(
            &json!({"mcpServers": {"a": {"url": "https://x/mcp"}, "b": "oops", "c": null}}),
        );
        assert_eq!(raw.0.keys().collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(raw.0["b"], json!("oops"));
    }

    #[test]
    fn http_entries_report_target_auth_kind_and_refs() {
        let u = "https://mcp.linear.app/mcp";
        let cases = [
            (Auth::None, "none", vec![]),
            (
                Auth::Bearer {
                    token: r("secret://T"),
                },
                "bearer",
                vec![r("secret://T")],
            ),
            (
                Auth::ClientCredentials {
                    token_url: "https://t/token".into(),
                    client_id: "id".into(),
                    client_secret: r("secret://CS"),
                    scope: None,
                },
                "client_credentials",
                vec![r("secret://CS")],
            ),
            (
                Auth::OAuth {
                    scopes: vec![],
                    client_id: None,
                    client_secret: Some(r("env://OAUTH_SECRET")),
                    client_metadata_url: None,
                },
                "oauth",
                vec![r("env://OAUTH_SECRET")],
            ),
        ];
        for (auth, kind, refs) in cases {
            let e = http(u, &[], auth);
            assert_eq!(
                (e.target(), e.auth_kind(), e.refs()),
                (u.to_string(), kind, refs)
            );
        }
        let e = http(
            u,
            &[
                ("X-API-Key", "secret://SEARCH_X_API_KEY"),
                ("X-Workspace", "acme"),
            ],
            Auth::None,
        );
        assert_eq!(e.auth_kind(), "header");
        assert_eq!(e.refs(), vec![r("secret://SEARCH_X_API_KEY")]);
        let e = http("secret://MCP_URL", &[("X-Key", "secret://K")], Auth::None);
        assert_eq!(e.target(), "secret://MCP_URL");
        assert_eq!(e.refs(), vec![r("secret://MCP_URL"), r("secret://K")]);
    }

    #[test]
    fn stdio_entries_report_the_command_line_as_written() {
        let env: BTreeMap<String, Text> = [
            ("GITHUB_TOKEN", "secret://PLAYWRIGHT_GITHUB_TOKEN"),
            ("HTTPS_PROXY", "env://HTTPS_PROXY"),
            ("DUP", "secret://ARG"),
            ("PLAYWRIGHT_BROWSERS_PATH", "/opt/pw-browsers"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), Text::parse(v)))
        .collect();
        let entry = Entry {
            name: "s".into(),
            disabled: false,
            transport: Transport::Stdio(StdioEntry {
                command: "npx".into(),
                args: ["-y", "@playwright/mcp@0.0.41", "secret://ARG"]
                    .map(Text::parse)
                    .to_vec(),
                env,
                cwd: None,
            }),
        };
        assert_eq!(entry.target(), "npx -y @playwright/mcp@0.0.41 secret://ARG");
        assert_eq!(entry.auth_kind(), "none");
        assert_eq!(
            entry.refs(),
            vec![
                r("secret://ARG"),
                r("secret://PLAYWRIGHT_GITHUB_TOKEN"),
                r("env://HTTPS_PROXY")
            ]
        );
    }
}
