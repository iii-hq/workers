use std::collections::BTreeMap;
use std::fmt;

use crate::functions::SecretChange;
use crate::host::{Host, SecretError};
use crate::server::{ErrorCode, ServerError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    Secret,
    Env,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub store: Store,
    pub name: String,
}

impl Ref {
    /// A reference is the whole string: `secret://NAME` or `env://NAME`, the
    /// scheme in any case, NAME matching `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`.
    pub fn parse(s: &str) -> Option<Ref> {
        let (scheme, name) = s.split_once("://")?;
        let store = if scheme.eq_ignore_ascii_case("secret") {
            Store::Secret
        } else if scheme.eq_ignore_ascii_case("env") {
            Store::Env
        } else {
            return None;
        };
        let mut chars = name.chars();
        let first = chars.next()?;
        let valid = (first.is_ascii_alphabetic() || first == '_')
            && name.len() <= 128
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        valid.then(|| Ref {
            store,
            name: name.into(),
        })
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = match self.store {
            Store::Secret => "secret",
            Store::Env => "env",
        };
        write!(f, "{scheme}://{}", self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Text {
    Literal(String),
    Ref(Ref),
}

impl Text {
    pub fn parse(s: &str) -> Text {
        Ref::parse(s).map_or_else(|| Text::Literal(s.into()), Text::Ref)
    }
}

/// The values of an entry's references, keyed by reference text. No `Debug`:
/// the values are secrets.
#[derive(Default)]
pub struct Resolved {
    pub values: BTreeMap<String, String>,
}

impl Resolved {
    pub fn text(&self, t: &Text) -> String {
        match t {
            Text::Literal(s) => s.clone(),
            Text::Ref(r) => self.values.get(&r.to_string()).cloned().unwrap_or_default(),
        }
    }

    /// Each value, and for one that parses as a URL also its normalized form:
    /// reqwest prints URLs that way (`https://h?k=v` as `https://h/?k=v`).
    pub fn secrets(&self) -> Vec<String> {
        let mut out = Vec::new();
        for value in self.values.values() {
            out.push(value.clone());
            if let Ok(url) = url::Url::parse(value)
                && url.as_str() != value
            {
                out.push(url.into());
            }
        }
        out
    }
}

/// Resolves each distinct reference once, in order, stopping at the first
/// failure.
pub async fn resolve_all(host: &dyn Host, refs: &[Ref]) -> Result<Resolved, ServerError> {
    let mut resolved = Resolved::default();
    for r in refs {
        let key = r.to_string();
        if resolved.values.contains_key(&key) {
            continue;
        }
        let value = host.resolve(&key).await.map_err(|e| match e {
            SecretError::NotFound => {
                ServerError::new(ErrorCode::SecretMissing, format!("`{key}` is not set"))
            }
            SecretError::Forbidden => ServerError::new(
                ErrorCode::SecretNotShared,
                format!("`{key}` is not shared with `mcp-client`"),
            ),
            SecretError::Unreachable(text) => ServerError::new(
                ErrorCode::ConnectFailed,
                format!("could not resolve `{key}`: {text}"),
            ),
        })?;
        resolved.values.insert(key, value);
    }
    Ok(resolved)
}

/// Whether a `secrets::on-changed` event names one of `refs`: by its `ref`
/// when that parses, else by its name in either store.
pub fn event_matches(event: &SecretChange, refs: &[Ref]) -> bool {
    if let Some(r) = event.reference.as_deref().and_then(Ref::parse) {
        return refs.contains(&r);
    }
    let name = event.name.as_deref();
    name.is_some_and(|name| refs.iter().any(|r| r.name == name))
}

/// Whether a header or env name probably carries a credential. The TypeScript
/// twin shares the name vector in its tests.
pub fn looks_secret_name(name: &str) -> bool {
    const NAMES: [&str; 3] = ["authorization", "cookie", "proxy-authorization"];
    const WORDS: [&str; 10] = [
        "KEY",
        "APIKEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "PAT",
        "AUTH",
        "CREDENTIAL",
        "CREDENTIALS",
    ];
    NAMES.iter().any(|n| name.eq_ignore_ascii_case(n))
        || name
            .split(['_', '-'])
            .any(|seg| WORDS.iter().any(|w| seg.eq_ignore_ascii_case(w)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_the_whole_string() {
        assert_eq!(
            Ref::parse("secret://LINEAR_API_KEY").unwrap(),
            Ref {
                store: Store::Secret,
                name: "LINEAR_API_KEY".into()
            }
        );
        assert_eq!(
            Ref::parse("ENV://HTTPS_PROXY").unwrap().to_string(),
            "env://HTTPS_PROXY"
        );
        assert_eq!(Ref::parse("Secret://_a.b-c").unwrap().name, "_a.b-c");
        assert!(Ref::parse(&format!("secret://A{}", "b".repeat(127))).is_some());
        assert!(Ref::parse(&format!("secret://A{}", "b".repeat(128))).is_none());
        for not_a_ref in [
            "Bearer secret://X",
            "secret://X ",
            " secret://X",
            "secret://",
            "secret://1X",
            "secret://A B",
            "secret://A/B",
            "vault://X",
            "secret:/X",
            "",
            "LINEAR_API_KEY",
            "https://x/secret://X",
        ] {
            assert!(Ref::parse(not_a_ref).is_none(), "{not_a_ref:?}");
        }
    }

    fn event(name: Option<&str>, reference: Option<&str>) -> SecretChange {
        SecretChange {
            name: name.map(String::from),
            reference: reference.map(String::from),
            action: Some("rotated".into()),
        }
    }

    #[test]
    fn event_matches_by_ref_and_falls_back_to_the_name() {
        let refs = [
            Ref::parse("secret://LINEAR_API_KEY").unwrap(),
            Ref::parse("env://HTTPS_PROXY").unwrap(),
        ];
        assert!(event_matches(
            &event(None, Some("SECRET://LINEAR_API_KEY")),
            &refs
        ));
        // The same name in the other store is another secret.
        assert!(!event_matches(
            &event(Some("LINEAR_API_KEY"), Some("env://LINEAR_API_KEY")),
            &refs
        ));
        // Without `ref`, the name matches in either store.
        assert!(event_matches(&event(Some("HTTPS_PROXY"), None), &refs));
        assert!(!event_matches(
            &event(Some("OTHER"), Some("secret://OTHER")),
            &refs
        ));
        assert!(!event_matches(&event(None, None), &refs));
    }

    #[test]
    fn text_is_a_reference_or_a_literal() {
        assert_eq!(
            Text::parse("env://HOME"),
            Text::Ref(Ref::parse("env://HOME").unwrap())
        );
        assert_eq!(
            Text::parse("Bearer secret://X"),
            Text::Literal("Bearer secret://X".into())
        );
    }

    #[test]
    fn secret_names() {
        for n in [
            "X-API-Key",
            "GITHUB_TOKEN",
            "Authorization",
            "cookie",
            "apikey",
            "MY_PAT",
            "DB_PASSWD",
            "X-Auth-Token",
            "CREDENTIALS_FILE",
            "proxy-authorization",
        ] {
            assert!(looks_secret_name(n), "{n}");
        }
        for n in [
            "X-Workspace",
            "PLAYWRIGHT_BROWSERS_PATH",
            "HTTPS_PROXY",
            "MONKEY",
            "AUTHOR",
        ] {
            assert!(!looks_secret_name(n), "{n}");
        }
    }
}
