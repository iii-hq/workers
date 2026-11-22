use crate::refs::{Ref, Text, looks_secret_name};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use url::{Host, Url};

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

/// One word as a POSIX shell reads it back: plain words as is, a word with a
/// control or bidi character as `$'…'`, any other in single quotes.
fn shell_word(s: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "@%+=:,./_-".contains(c);
    let hidden = |c: char| {
        c.is_control()
            || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{2069}' | '\u{feff}')
    };
    if !s.is_empty() && s.chars().all(plain) {
        return s.into();
    }
    if !s.chars().any(hidden) {
        return format!("'{}'", s.replace('\'', r"'\''"));
    }
    let mut out = String::from("$'");
    for c in s.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '\'' => out.push_str(r"\'"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c if hidden(c) => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
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

    /// The URL, or the command line quoted as a shell would need it,
    /// references as written.
    pub fn target(&self) -> String {
        match &self.transport {
            Transport::Http(h) => text_str(&h.url),
            Transport::Stdio(s) => std::iter::once(shell_word(&s.command))
                .chain(s.args.iter().map(|a| shell_word(&text_str(a))))
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

/// The whole message of rule 9, so callers can tell it from `invalid_config`.
pub const STDIO_OFF: &str =
    "stdio servers are turned off on this worker (III_MCP_CLIENT_STDIO=off)";

const REF_ONLY: &str = "must be a reference (`secret://NAME` or `env://NAME`), not a literal";
const NO_LITERAL: &str =
    "a literal is not allowed here; store the value in the vault and use `secret://NAME`";
const OBJECT: &str = "must be an object";
const STRING: &str = "must be a string";
const LOOPBACK: &str = "must be https, or http on a loopback host";
const TRANSPORT_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "content-type",
    "accept",
    "last-event-id",
];
const FINGERPRINT_KEYS: [&str; 7] = ["command", "args", "env", "cwd", "url", "headers", "auth"];

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Issue {
    /// JSON pointer into the candidate value.
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SendEffect {
    #[serde(rename = "ref")]
    pub reference: String,
    pub host: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
pub struct Effects {
    pub runs: Option<String>,
    pub sends: Vec<SendEffect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    /// `Some` exactly when `errors` is empty.
    pub entry: Option<Entry>,
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
    pub fingerprint: String,
    pub effects: Effects,
}

fn esc(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

struct Sink {
    base: String,
    errors: Vec<Issue>,
    warnings: Vec<Issue>,
}

impl Sink {
    fn err(&mut self, at: &str, message: &str) {
        self.errors.push(Issue {
            path: format!("{}{at}", self.base),
            message: message.into(),
        });
    }

    fn warn(&mut self, at: &str, message: &str) {
        self.warnings.push(Issue {
            path: format!("{}{at}", self.base),
            message: message.into(),
        });
    }
}

pub(crate) fn is_loopback(host: Option<Host<&str>>) -> bool {
    match host {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(a)) => a.octets()[0] == 127,
        Some(Host::Ipv6(a)) => a.is_loopback(),
        None => false,
    }
}

/// The URL checks shared by rule 3 and by a URL that came out of a reference.
pub fn check_url_text(text: &str) -> Result<(), &'static str> {
    let u = Url::parse(text).map_err(|_| "not a valid URL")?;
    if !(u.scheme() == "https" || u.scheme() == "http" && is_loopback(u.host())) {
        return Err(LOOPBACK);
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("must not carry a user name or password");
    }
    Ok(())
}

fn url_issue(text: &str, https_only: bool) -> Option<String> {
    if https_only && Url::parse(text).is_ok_and(|u| u.scheme() != "https") {
        return Some("must be https".into());
    }
    if let Err(m) = check_url_text(text) {
        return Some(m.into());
    }
    let u = Url::parse(text).ok()?;
    u.query_pairs().find(|(k, _)| looks_secret_name(k)).map(|(k, _)| {
        format!(
            "query parameter `{k}` looks like a secret; make the whole value a reference (`secret://NAME`)"
        )
    })
}

fn url_field(sink: &mut Sink, at: &str, v: Option<&Value>, https_only: bool, ref_ok: bool) {
    match v.and_then(Value::as_str) {
        None => sink.err(at, STRING),
        Some(s) if ref_ok && Ref::parse(s).is_some() => {}
        Some(s) => {
            if let Some(m) = url_issue(s, https_only) {
                sink.err(at, &m);
            }
        }
    }
}

fn is_ref(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).and_then(Ref::parse).is_some()
}

fn present<'a>(o: &'a serde_json::Map<String, Value>, k: &str) -> Option<&'a Value> {
    o.get(k).filter(|v| !v.is_null())
}

fn string_array(v: &Value) -> Option<Vec<&str>> {
    v.as_array()?.iter().map(Value::as_str).collect()
}

fn auth_known(a: &serde_json::Map<String, Value>) -> Option<&'static [&'static str]> {
    Some(match a.get("type").and_then(Value::as_str)? {
        "bearer" => &["type", "token"],
        "client_credentials" => &["type", "token_url", "client_id", "client_secret", "scope"],
        "oauth" => &[
            "type",
            "scopes",
            "client_id",
            "client_secret",
            "client_metadata_url",
        ],
        _ => return None,
    })
}

/// Rule 3 on the auth URLs.
fn check_auth_urls(sink: &mut Sink, a: &serde_json::Map<String, Value>) {
    match a.get("type").and_then(Value::as_str) {
        Some("client_credentials") => {
            if let v @ Some(_) = present(a, "token_url") {
                url_field(sink, "/auth/token_url", v, false, false);
            }
        }
        Some("oauth") => {
            if let v @ Some(_) = present(a, "client_metadata_url") {
                url_field(sink, "/auth/client_metadata_url", v, true, false);
            }
        }
        _ => {}
    }
}

/// Rule 4.
fn check_auth(sink: &mut Sink, auth: &Value) {
    let Some(a) = auth.as_object() else {
        return sink.err("/auth", OBJECT);
    };
    match a.get("type").and_then(Value::as_str) {
        Some("bearer") => {
            if !is_ref(a.get("token")) {
                sink.err("/auth/token", REF_ONLY);
            }
        }
        Some("client_credentials") => {
            if !is_ref(a.get("client_secret")) {
                sink.err("/auth/client_secret", REF_ONLY);
            }
            if present(a, "token_url").is_none() {
                sink.err("/auth/token_url", "required for client_credentials");
            }
            match present(a, "client_id") {
                None => sink.err("/auth/client_id", "required for client_credentials"),
                Some(v) if !v.is_string() => sink.err("/auth/client_id", STRING),
                Some(_) => {}
            }
            if present(a, "scope").is_some_and(|v| !v.is_string()) {
                sink.err("/auth/scope", STRING);
            }
        }
        Some("oauth") => {
            if present(a, "client_secret").is_some() && !is_ref(a.get("client_secret")) {
                sink.err("/auth/client_secret", REF_ONLY);
            }
            if present(a, "client_id").is_some_and(|v| !v.is_string()) {
                sink.err("/auth/client_id", STRING);
            }
            if present(a, "scopes").is_some_and(|v| string_array(v).is_none()) {
                sink.err("/auth/scopes", "must be an array of strings");
            }
        }
        _ => sink.err(
            "/auth/type",
            "unknown auth type; use `bearer`, `client_credentials` or `oauth`",
        ),
    }
}

fn reserved_header(lower: &str) -> bool {
    TRANSPORT_HEADERS.contains(&lower) || lower.starts_with("mcp-")
}

/// Rule 5.
fn check_headers(sink: &mut Sink, headers: &Value, has_auth: bool) {
    let Some(h) = headers.as_object() else {
        return sink.err("/headers", OBJECT);
    };
    for (name, v) in h {
        let at = format!("/headers/{}", esc(name));
        let lower = name.to_ascii_lowercase();
        if http::HeaderName::from_bytes(name.as_bytes()).is_err() {
            sink.err(&at, "not a valid HTTP header name");
        } else if reserved_header(&lower) {
            sink.err(&at, "set by the transport, cannot be configured");
        } else if lower == "authorization" && has_auth {
            sink.err(
                &at,
                "remove the `Authorization` header or the `auth` block; both cannot be set",
            );
        } else if !v.is_string() {
            sink.err(&at, STRING);
        }
    }
}

/// Rule 6: a literal under a name that looks like a credential.
fn check_literals(sink: &mut Sink, o: &serde_json::Map<String, Value>, http: bool, has_auth: bool) {
    let (key, section) = if http {
        ("headers", "/headers")
    } else {
        ("env", "/env")
    };
    let Some(m) = present(o, key).and_then(Value::as_object) else {
        return;
    };
    for (name, v) in m {
        let lower = name.to_ascii_lowercase();
        let skipped = http
            && (http::HeaderName::from_bytes(name.as_bytes()).is_err()
                || reserved_header(&lower)
                || lower == "authorization" && has_auth);
        if !skipped && v.is_string() && !is_ref(Some(v)) && looks_secret_name(name) {
            sink.err(&format!("{section}/{}", esc(name)), NO_LITERAL);
        }
    }
}

/// Rule 7 for a stdio entry.
fn check_stdio(sink: &mut Sink, o: &serde_json::Map<String, Value>) {
    if !o
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| !c.trim().is_empty())
    {
        sink.err("/command", "must be a non-empty string");
    }
    if present(o, "args").is_some_and(|v| string_array(v).is_none()) {
        sink.err("/args", "must be an array of strings");
    }
    if present(o, "cwd").is_some_and(|v| {
        !v.as_str()
            .is_some_and(|s| std::path::Path::new(s).is_absolute())
    }) {
        sink.err("/cwd", "must be an absolute path");
    }
    match present(o, "env") {
        None => {}
        Some(Value::Object(e)) => {
            for name in e.iter().filter(|(_, v)| !v.is_string()).map(|(k, _)| k) {
                sink.err(&format!("/env/{}", esc(name)), STRING);
            }
        }
        Some(_) => sink.err("/env", OBJECT),
    }
    if present(o, "auth").is_some() {
        sink.err(
            "/auth",
            "auth is for http servers; a stdio server takes credentials through `env`",
        );
    }
}

/// Rule 8: any string anywhere that the configuration worker would expand.
fn walk_expansions(sink: &mut Sink, v: &Value, at: &str) {
    match v {
        Value::String(s) => {
            if s.contains("${") {
                sink.err(at, "use `env://VAR` instead of `${VAR}`");
            }
        }
        Value::Array(a) => a
            .iter()
            .enumerate()
            .for_each(|(i, x)| walk_expansions(sink, x, &format!("{at}/{i}"))),
        Value::Object(o) => o
            .iter()
            .for_each(|(k, x)| walk_expansions(sink, x, &format!("{at}/{}", esc(k)))),
        _ => {}
    }
}

fn valid_name(name: &str) -> bool {
    (1..=24).contains(&name.len())
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn build_auth(v: Option<&Value>) -> Option<Auth> {
    let Some(a) = v.and_then(Value::as_object) else {
        return Some(Auth::None);
    };
    let s = |k: &str| present(a, k).and_then(Value::as_str).map(String::from);
    let r = |k: &str| present(a, k).and_then(Value::as_str).and_then(Ref::parse);
    Some(match a.get("type")?.as_str()? {
        "bearer" => Auth::Bearer { token: r("token")? },
        "client_credentials" => Auth::ClientCredentials {
            token_url: s("token_url")?,
            client_id: s("client_id")?,
            client_secret: r("client_secret")?,
            scope: s("scope"),
        },
        _ => Auth::OAuth {
            scopes: present(a, "scopes")
                .and_then(string_array)
                .map(|v| v.into_iter().map(String::from).collect())
                .unwrap_or_default(),
            client_id: s("client_id"),
            client_secret: r("client_secret"),
            client_metadata_url: s("client_metadata_url"),
        },
    })
}

fn texts(o: Option<&Value>) -> Option<BTreeMap<String, Text>> {
    let Some(o) = present_value(o) else {
        return Some(BTreeMap::new());
    };
    o.as_object()?
        .iter()
        .map(|(k, v)| Some((k.clone(), Text::parse(v.as_str()?))))
        .collect()
}

fn present_value(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null())
}

fn build(name: &str, o: &serde_json::Map<String, Value>, http: bool) -> Option<Entry> {
    let transport = if http {
        Transport::Http(HttpEntry {
            url: Text::parse(o.get("url")?.as_str()?),
            headers: texts(o.get("headers"))?,
            auth: build_auth(present(o, "auth"))?,
        })
    } else {
        Transport::Stdio(StdioEntry {
            command: o.get("command")?.as_str()?.into(),
            args: present(o, "args")
                .map_or(Some(vec![]), string_array)?
                .into_iter()
                .map(Text::parse)
                .collect(),
            env: texts(o.get("env"))?,
            cwd: present(o, "cwd").and_then(Value::as_str).map(PathBuf::from),
        })
    };
    Some(Entry {
        name: name.into(),
        disabled: o.get("disabled").and_then(Value::as_bool).unwrap_or(false),
        transport,
    })
}

fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default()
}

fn effects(e: &Entry) -> Effects {
    let mut sends: Vec<SendEffect> = Vec::new();
    let mut add = |r: &Ref, host: &str| {
        let row = SendEffect {
            reference: r.to_string(),
            host: host.into(),
        };
        if !sends.contains(&row) {
            sends.push(row);
        }
    };
    let runs = match &e.transport {
        Transport::Stdio(s) => {
            for t in s.args.iter().chain(s.env.values()) {
                if let Text::Ref(r) = t {
                    add(r, "process");
                }
            }
            let cd = s.cwd.as_ref().map(|d| {
                let dir = d.to_string_lossy();
                format!("cd {} && ", shell_word(&dir))
            });
            let env = s
                .env
                .iter()
                .map(|(k, v)| format!("{}={} ", shell_word(k), shell_word(&text_str(v))));
            Some(cd.into_iter().chain(env).collect::<String>() + &e.target())
        }
        Transport::Http(h) => {
            let url_host = match &h.url {
                Text::Ref(r) => r.to_string(),
                Text::Literal(s) => host_of(s),
            };
            for t in std::iter::once(&h.url).chain(h.headers.values()) {
                if let Text::Ref(r) = t {
                    add(r, &url_host);
                }
            }
            match &h.auth {
                Auth::Bearer { token } => add(token, &url_host),
                Auth::ClientCredentials {
                    token_url,
                    client_secret,
                    ..
                } => add(client_secret, &host_of(token_url)),
                Auth::OAuth {
                    client_secret: Some(s),
                    ..
                } => add(s, &url_host),
                _ => {}
            }
            None
        }
    };
    Effects { runs, sends }
}

/// SHA-256 hex over the canonical JSON of the seven keys an operator approves.
pub fn fingerprint(raw: &Value) -> String {
    let picked: serde_json::Map<String, Value> = FINGERPRINT_KEYS
        .iter()
        .filter_map(|k| Some((k.to_string(), raw.get(k)?.clone())))
        .collect();
    // serde_json::Map is a BTreeMap here, so keys come out sorted at every level.
    let text = serde_json::to_string(&picked).unwrap_or_default();
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn check_entry(name: &str, raw: &Value, stdio_enabled: bool) -> Checked {
    let mut sink = Sink {
        base: format!("/mcpServers/{}", esc(name)),
        errors: vec![],
        warnings: vec![],
    };
    let mut entry = None;
    if let Some(o) = raw.as_object() {
        if !valid_name(name) {
            sink.err(
                "",
                "server name must be 1 to 24 characters: lower-case letters, digits and hyphens, not starting with a hyphen",
            );
        }
        let http = transport_kind(&mut sink, o);
        if let Some(http) = http {
            check_transport(&mut sink, o, http);
        }
        walk_expansions(&mut sink, raw, "");
        if http == Some(false) && !stdio_enabled {
            sink.err("", STDIO_OFF);
        }
        warn_unknown(&mut sink, o, http);
        if sink.errors.is_empty() {
            entry = http.and_then(|h| build(name, o, h));
        }
    } else {
        sink.err("", "entry must be an object");
    }
    let effects = entry.as_ref().map(effects).unwrap_or_default();
    Checked {
        entry,
        errors: sink.errors,
        warnings: sink.warnings,
        fingerprint: fingerprint(raw),
        effects,
    }
}

/// Rule 2; `Some(true)` is an http entry, `Some(false)` stdio, `None` an error.
fn transport_kind(sink: &mut Sink, o: &serde_json::Map<String, Value>) -> Option<bool> {
    let (url, command) = (present(o, "url").is_some(), present(o, "command").is_some());
    let http = match (url, command) {
        (true, true) => {
            sink.err("", "set `url` or `command`, not both");
            return None;
        }
        (false, false) => {
            sink.err("", "set `url` (http) or `command` (stdio)");
            return None;
        }
        _ => url,
    };
    let declared = present(o, "type").map(|t| t.as_str().unwrap_or(""));
    match declared {
        None => Some(http),
        Some("http" | "streamable-http") if !http => {
            sink.err("/type", "type `http` needs `url`");
            None
        }
        Some("stdio") if http => {
            sink.err("/type", "type `stdio` needs `command`");
            None
        }
        Some("http" | "streamable-http" | "stdio") => Some(http),
        Some(t @ ("sse" | "ws")) => {
            sink.err(
                "/type",
                &format!("type `{t}` is not supported; use `http` or `stdio`"),
            );
            None
        }
        Some(_) => {
            sink.err("/type", "unknown type; use `http` or `stdio`");
            None
        }
    }
}

/// Rules 3 to 7 once the transport is known.
fn check_transport(sink: &mut Sink, o: &serde_json::Map<String, Value>, http: bool) {
    let auth = present(o, "auth");
    if http {
        url_field(sink, "/url", o.get("url"), false, true);
        if let Some(a) = auth.and_then(Value::as_object) {
            check_auth_urls(sink, a);
        }
        if let Some(a) = auth {
            check_auth(sink, a);
        }
        if let Some(h) = present(o, "headers") {
            check_headers(sink, h, auth.is_some());
        }
    }
    check_literals(sink, o, http, auth.is_some());
    if !http {
        check_stdio(sink, o);
    }
    if present(o, "disabled").is_some_and(|v| !v.is_boolean()) {
        sink.err("/disabled", "must be true or false");
    }
}

/// Rule 10; the "ignored on" warnings need a known transport.
fn warn_unknown(sink: &mut Sink, o: &serde_json::Map<String, Value>, http: Option<bool>) {
    for k in o.keys() {
        let at = format!("/{}", esc(k));
        if !KNOWN_FIELDS.contains(&k.as_str()) {
            sink.warn(&at, "unknown field, ignored");
        } else if http == Some(true) && ["args", "env", "cwd"].contains(&k.as_str()) {
            sink.warn(&at, "ignored on an http entry");
        } else if http == Some(false) && k == "headers" {
            sink.warn(&at, "ignored on a stdio entry");
        }
    }
    if http == Some(true)
        && let Some(a) = present(o, "auth").and_then(Value::as_object)
        && let Some(known) = auth_known(a)
    {
        for k in a.keys().filter(|k| !known.contains(&k.as_str())) {
            sink.warn(&format!("/auth/{}", esc(k)), "unknown field, ignored");
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

    const REF_ONLY: &str = "must be a reference (`secret://NAME` or `env://NAME`), not a literal";
    const NO_LITERAL: &str =
        "a literal is not allowed here; store the value in the vault and use `secret://NAME`";
    const URL: &str = "https://x.example/mcp";
    const LOOPBACK_MSG: &str = "must be https, or http on a loopback host";

    fn check(name: &str, raw: Value) -> Checked {
        check_entry(name, &raw, true)
    }

    fn one_error(raw: Value) -> (String, String) {
        let c = check("s", raw.clone());
        assert!(c.entry.is_none(), "{raw}");
        assert_eq!(c.errors.len(), 1, "{raw}: {:?}", c.errors);
        let e = &c.errors[0];
        (
            e.path.strip_prefix("/mcpServers/s").unwrap().to_string(),
            e.message.clone(),
        )
    }

    fn send(r: &str, host: &str) -> SendEffect {
        SendEffect {
            reference: r.into(),
            host: host.into(),
        }
    }

    #[test]
    fn the_standard_shapes_parse() {
        let cases = [
            (
                json!({"type": "http", "url": "https://mcp.linear.app/mcp",
                    "auth": {"type": "bearer", "token": "secret://LINEAR_API_KEY"}}),
                "bearer",
            ),
            (json!({"url": URL}), "none"),
            (
                json!({"type": "streamable-http", "url": "http://127.0.0.1:8080/mcp"}),
                "none",
            ),
            (
                json!({"url": "http://localhost:8080/mcp", "disabled": true}),
                "none",
            ),
            (
                json!({"url": "http://[::1]:8080/mcp",
                    "headers": {"X-API-Key": "secret://K", "X-Workspace": "acme"}}),
                "header",
            ),
            (json!({"url": "secret://MCP_URL"}), "none"),
            (json!({"url": URL, "auth": {"type": "oauth"}}), "oauth"),
            (
                json!({"url": URL, "auth": {"type": "client_credentials",
                    "token_url": "https://api.linear.app/oauth/token", "client_id": "id",
                    "client_secret": "secret://S", "scope": "read"}}),
                "client_credentials",
            ),
        ];
        for (raw, kind) in cases {
            let c = check("linear", raw.clone());
            assert!(c.errors.is_empty() && c.warnings.is_empty(), "{raw}: {c:?}");
            assert_eq!(c.entry.unwrap().auth_kind(), kind, "{raw}");
        }
        let c = check(
            "linear",
            json!({"url": "http://localhost:8080/mcp", "disabled": true}),
        );
        let e = c.entry.unwrap();
        assert!(e.disabled && e.name == "linear");
        let c = check(
            "pw",
            json!({"command": "npx", "args": ["-y", "@playwright/mcp@0.0.41"],
                "env": {"GITHUB_TOKEN": "secret://T", "HTTPS_PROXY": "env://HTTPS_PROXY",
                    "PLAYWRIGHT_BROWSERS_PATH": "/opt/pw-browsers"},
                "cwd": "/srv"}),
        );
        assert!(c.errors.is_empty() && c.warnings.is_empty(), "{c:?}");
        assert!(matches!(c.entry.unwrap().transport, Transport::Stdio(_)));
    }

    #[test]
    fn every_rule_refuses_its_case() {
        let url = URL;
        let cc = |f: Value| {
            let mut a = json!({"type": "client_credentials",
                "token_url": "https://t.example/token", "client_id": "id",
                "client_secret": "secret://S"});
            for (k, v) in f.as_object().unwrap() {
                if v.is_null() {
                    a.as_object_mut().unwrap().remove(k);
                } else {
                    a[k] = v.clone();
                }
            }
            json!({"url": url, "auth": a})
        };
        let both = "set `url` or `command`, not both";
        let cases: Vec<(Value, &str, &str)> = vec![
            (json!(null), "", "entry must be an object"),
            (
                json!({"type": "http"}),
                "",
                "set `url` (http) or `command` (stdio)",
            ),
            (json!({"url": url, "command": "npx"}), "", both),
            (
                json!({"type": "sse", "url": url}),
                "/type",
                "type `sse` is not supported; use `http` or `stdio`",
            ),
            (
                json!({"type": "ws", "url": url}),
                "/type",
                "type `ws` is not supported; use `http` or `stdio`",
            ),
            (
                json!({"type": "grpc", "url": url}),
                "/type",
                "unknown type; use `http` or `stdio`",
            ),
            (
                json!({"type": "stdio", "url": url}),
                "/type",
                "type `stdio` needs `command`",
            ),
            (
                json!({"type": "http", "command": "npx"}),
                "/type",
                "type `http` needs `url`",
            ),
            (json!({"url": 5}), "/url", "must be a string"),
            (json!({"url": "not a url"}), "/url", "not a valid URL"),
            (
                json!({"url": "ftp://x.example/mcp"}),
                "/url",
                "must be https, or http on a loopback host",
            ),
            (
                json!({"url": "http://example.com/mcp"}),
                "/url",
                "must be https, or http on a loopback host",
            ),
            (
                json!({"url": "https://u:p@x.example/mcp"}),
                "/url",
                "must not carry a user name or password",
            ),
            (
                json!({"url": "https://x.example/mcp?team=a&api_key=1"}),
                "/url",
                "query parameter `api_key` looks like a secret; make the whole value a reference (`secret://NAME`)",
            ),
            (
                cc(json!({"token_url": "http://example.com/t"})),
                "/auth/token_url",
                "must be https, or http on a loopback host",
            ),
            (
                json!({"url": url, "auth": {"type": "oauth", "client_metadata_url": "http://localhost/c.json"}}),
                "/auth/client_metadata_url",
                "must be https",
            ),
            (
                json!({"url": url, "auth": "x"}),
                "/auth",
                "must be an object",
            ),
            (
                json!({"url": url, "auth": {"type": "basic"}}),
                "/auth/type",
                "unknown auth type; use `bearer`, `client_credentials` or `oauth`",
            ),
            (
                json!({"url": url, "auth": {"type": "bearer", "token": "abc"}}),
                "/auth/token",
                REF_ONLY,
            ),
            (
                json!({"url": url, "auth": {"type": "bearer", "token": "Bearer secret://X"}}),
                "/auth/token",
                REF_ONLY,
            ),
            (
                json!({"url": url, "auth": {"type": "bearer"}}),
                "/auth/token",
                REF_ONLY,
            ),
            (
                cc(json!({"client_secret": "abc"})),
                "/auth/client_secret",
                REF_ONLY,
            ),
            (
                cc(json!({"client_secret": null})),
                "/auth/client_secret",
                REF_ONLY,
            ),
            (
                cc(json!({"token_url": null})),
                "/auth/token_url",
                "required for client_credentials",
            ),
            (
                cc(json!({"client_id": null})),
                "/auth/client_id",
                "required for client_credentials",
            ),
            (
                cc(json!({"client_id": 5})),
                "/auth/client_id",
                "must be a string",
            ),
            (cc(json!({"scope": 5})), "/auth/scope", "must be a string"),
            (
                json!({"url": url, "auth": {"type": "oauth", "scopes": "x"}}),
                "/auth/scopes",
                "must be an array of strings",
            ),
            (
                json!({"url": url, "auth": {"type": "oauth", "client_secret": "abc"}}),
                "/auth/client_secret",
                REF_ONLY,
            ),
            (
                json!({"url": url, "headers": "x"}),
                "/headers",
                "must be an object",
            ),
            (
                json!({"url": url, "headers": {"bad name": "x"}}),
                "/headers/bad name",
                "not a valid HTTP header name",
            ),
            (
                json!({"url": url, "headers": {"Content-Type": "x"}}),
                "/headers/Content-Type",
                "set by the transport, cannot be configured",
            ),
            (
                json!({"url": url, "headers": {"Mcp-Session-Id": "x"}}),
                "/headers/Mcp-Session-Id",
                "set by the transport, cannot be configured",
            ),
            (
                json!({"url": url, "headers": {"Last-Event-ID": "x"}}),
                "/headers/Last-Event-ID",
                "set by the transport, cannot be configured",
            ),
            (
                json!({"url": url, "headers": {"Authorization": "secret://A"}, "auth": {"type": "oauth"}}),
                "/headers/Authorization",
                "remove the `Authorization` header or the `auth` block; both cannot be set",
            ),
            (
                json!({"url": url, "headers": {"X-Workspace": 5}}),
                "/headers/X-Workspace",
                "must be a string",
            ),
            (
                json!({"url": url, "headers": {"Authorization": "Bearer abc"}}),
                "/headers/Authorization",
                NO_LITERAL,
            ),
            (
                json!({"command": "npx", "env": {"GITHUB_TOKEN": "abc"}}),
                "/env/GITHUB_TOKEN",
                NO_LITERAL,
            ),
            (
                json!({"command": ""}),
                "/command",
                "must be a non-empty string",
            ),
            (
                json!({"command": 5}),
                "/command",
                "must be a non-empty string",
            ),
            (
                json!({"command": "npx", "args": "x"}),
                "/args",
                "must be an array of strings",
            ),
            (
                json!({"command": "npx", "args": [1]}),
                "/args",
                "must be an array of strings",
            ),
            (
                json!({"command": "npx", "cwd": "rel/dir"}),
                "/cwd",
                "must be an absolute path",
            ),
            (
                json!({"command": "npx", "env": "x"}),
                "/env",
                "must be an object",
            ),
            (
                json!({"command": "npx", "env": {"A": 1}}),
                "/env/A",
                "must be a string",
            ),
            (
                json!({"command": "npx", "auth": {"type": "oauth"}}),
                "/auth",
                "auth is for http servers; a stdio server takes credentials through `env`",
            ),
            (
                json!({"url": url, "disabled": "yes"}),
                "/disabled",
                "must be true or false",
            ),
            (
                json!({"command": "npx", "args": ["--dir=${HOME}/data"]}),
                "/args/0",
                "use `env://VAR` instead of `${VAR}`",
            ),
            (
                json!({"command": "npx", "args": ["--dir=${HOME"]}),
                "/args/0",
                "use `env://VAR` instead of `${VAR}`",
            ),
            (
                json!({"url": url, "note": {"deep": ["${X}"]}}),
                "/note/deep/0",
                "use `env://VAR` instead of `${VAR}`",
            ),
        ];
        for (raw, path, message) in cases {
            assert_eq!(
                one_error(raw.clone()),
                (path.to_string(), message.to_string()),
                "{raw}"
            );
        }
    }

    #[test]
    fn names_follow_rule_one_and_errors_keep_the_rule_order() {
        let raw = json!({"url": "https://x.example/${T}/mcp"});
        for ok in ["linear", "a", "0", "linear-app", &"a".repeat(24)] {
            let c = check(ok, json!({"url": URL}));
            assert!(c.errors.is_empty(), "{ok}");
        }
        for bad in ["", "Linear", "-a", "a_b", "a.b", &"a".repeat(25)] {
            let c = check(bad, raw.clone());
            assert_eq!(c.errors.len(), 2, "{bad:?}: {:?}", c.errors);
            assert_eq!(c.errors[0].path, format!("/mcpServers/{bad}"));
            assert!(
                c.errors[0].message.starts_with("server name must be"),
                "{bad:?}"
            );
            assert!(c.errors[1].message.contains("env://VAR"), "{bad:?}");
        }
    }

    #[test]
    fn stdio_can_be_switched_off() {
        assert_eq!(
            check_entry("s", &json!({"command": "npx"}), false).errors,
            vec![Issue {
                path: "/mcpServers/s".into(),
                message: STDIO_OFF.into()
            }]
        );
        assert!(
            check_entry("s", &json!({"url": URL}), false)
                .entry
                .is_some()
        );
    }

    #[test]
    fn unknown_fields_are_warnings_and_never_read() {
        let c = check(
            "s",
            json!({"url": URL, "headersHelper": "/bin/steal", "http_headers_helper": "x",
                "timeout": 30, "env": {"A": "b"}, "auth": {"type": "oauth", "audience": "x"}}),
        );
        assert!(c.entry.is_some(), "{c:?}");
        let mut got: Vec<_> = c
            .warnings
            .iter()
            .map(|w| (w.path.as_str(), w.message.as_str()))
            .collect();
        got.sort();
        let unknown = "unknown field, ignored";
        assert_eq!(
            got,
            vec![
                ("/mcpServers/s/auth/audience", unknown),
                ("/mcpServers/s/env", "ignored on an http entry"),
                ("/mcpServers/s/headersHelper", unknown),
                ("/mcpServers/s/http_headers_helper", unknown),
                ("/mcpServers/s/timeout", unknown),
            ]
        );
        let c = check("s", json!({"command": "npx", "headers": {"X": "y"}}));
        assert_eq!(c.warnings.len(), 1);
        assert_eq!(c.warnings[0].message, "ignored on a stdio entry");
    }

    #[test]
    fn fingerprint_covers_seven_keys_whatever_their_order() {
        const LINEAR_FP: &str = "4ba127a86d69977e896f4e4291cf18c8dac1f1bc7c7cf055bdd20216efa50ba6";
        let base = json!({"type": "http", "url": "https://mcp.linear.app/mcp",
            "auth": {"type": "bearer", "token": "secret://LINEAR_API_KEY"}});
        assert_eq!(fingerprint(&base), LINEAR_FP);
        let reordered = json!({"auth": {"token": "secret://LINEAR_API_KEY", "type": "bearer"},
            "url": "https://mcp.linear.app/mcp"});
        assert_eq!(fingerprint(&reordered), LINEAR_FP);
        let mut outside = base.clone();
        outside["disabled"] = json!(true);
        outside["type"] = json!("streamable-http");
        outside["note"] = json!("x");
        assert_eq!(fingerprint(&outside), LINEAR_FP);
        for (k, v) in [
            ("command", json!("npx")),
            ("args", json!(["a"])),
            ("env", json!({"A": "b"})),
            ("cwd", json!("/x")),
            ("url", json!("https://other.example/mcp")),
            ("headers", json!({"A": "b"})),
            ("auth", json!({"type": "oauth"})),
        ] {
            let mut changed = base.clone();
            changed[k] = v;
            assert_ne!(fingerprint(&changed), LINEAR_FP, "{k}");
        }
        assert_eq!(fingerprint(&json!("oops")), fingerprint(&json!({})));
        assert_eq!(check("linear", base).fingerprint, LINEAR_FP);
    }

    #[test]
    fn effects_say_what_approving_allows() {
        let linear = json!({"type": "http", "url": "https://mcp.linear.app/mcp",
            "headers": {"X-Key": "env://K"},
            "auth": {"type": "bearer", "token": "secret://LINEAR_API_KEY"}});
        let e = check("linear", linear).effects;
        assert_eq!(e.runs, None);
        assert_eq!(
            e.sends,
            vec![
                send("env://K", "mcp.linear.app"),
                send("secret://LINEAR_API_KEY", "mcp.linear.app")
            ]
        );
        let cc = json!({"url": "https://mcp.linear.app/mcp", "auth": {"type": "client_credentials",
            "token_url": "https://api.linear.app/oauth/token", "client_id": "id",
            "client_secret": "secret://S"}});
        assert_eq!(
            check("s", cc).effects.sends,
            vec![send("secret://S", "api.linear.app")]
        );
        let by_ref = json!({"url": "secret://MCP_URL", "headers": {"X-Key": "secret://K"}});
        assert_eq!(
            check("s", by_ref).effects.sends,
            vec![
                send("secret://MCP_URL", "secret://MCP_URL"),
                send("secret://K", "secret://MCP_URL")
            ]
        );
        let stdio = json!({"command": "npx", "args": ["-y", "@playwright/mcp@0.0.41"],
            "env": {"GITHUB_TOKEN": "secret://T"}});
        let e = check("s", stdio).effects;
        assert_eq!(
            e.runs.as_deref(),
            Some("GITHUB_TOKEN=secret://T npx -y @playwright/mcp@0.0.41")
        );
        assert_eq!(e.sends, vec![send("secret://T", "process")]);
        assert_eq!(
            check("s", json!({"url": "ftp://x"})).effects,
            Effects::default()
        );
        assert_eq!(
            serde_json::to_value(send("secret://T", "process")).unwrap(),
            json!({"ref": "secret://T", "host": "process"})
        );
    }

    #[test]
    fn the_run_line_shows_env_and_cwd_and_quotes_every_word() {
        let runs = |raw: Value| check("s", raw).effects.runs.unwrap();
        let path = json!({"command": "node", "args": ["server.js"],
            "env": {"PATH": "/tmp/x:/usr/bin"}});
        let other_path = json!({"command": "node", "args": ["server.js"],
            "env": {"PATH": "/usr/bin"}});
        assert_eq!(runs(path.clone()), "PATH=/tmp/x:/usr/bin node server.js");
        assert_ne!(runs(path), runs(other_path));
        let node_options = json!({"command": "node", "args": ["server.js"],
            "env": {"NODE_OPTIONS": "--require /tmp/x.js", "TOKEN": "secret://T"}});
        assert_eq!(
            runs(node_options),
            "NODE_OPTIONS='--require /tmp/x.js' TOKEN=secret://T node server.js"
        );
        let cwd = json!({"command": "node", "args": ["server.js"], "cwd": "/srv/a b"});
        let other_cwd = json!({"command": "node", "args": ["server.js"], "cwd": "/srv/c"});
        assert_eq!(runs(cwd.clone()), "cd '/srv/a b' && node server.js");
        assert_ne!(runs(cwd), runs(other_cwd));
        assert_eq!(
            runs(json!({"command": "node", "args": ["server.js"]})),
            "node server.js"
        );

        let target = |args: &[&str]| {
            check("s", json!({"command": "sh", "args": args}))
                .entry
                .unwrap()
                .target()
        };
        assert_ne!(
            target(&["-c", "echo `id`"]),
            target(&["-c", "echo", "`id`"])
        );
        assert_eq!(target(&["-c", "echo `id`"]), "sh -c 'echo `id`'");
        assert_eq!(
            target(&["a b", "it's", "a\nb", "x\u{202e}y", ""]),
            r"sh 'a b' 'it'\''s' $'a\nb' $'x\u202ey' ''"
        );
        assert_eq!(
            target(&["tab\there", "back\\slash'"]),
            r"sh $'tab\there' 'back\slash'\'''"
        );
        assert_eq!(target(&["bell\u{7}'\\"]), r"sh $'bell\u0007\'\\'");
    }

    #[test]
    fn unknown_fields_warn_even_after_a_transport_error() {
        let c = check(
            "s",
            json!({"type": "sse", "url": URL, "headersHelper": "x"}),
        );
        assert_eq!(c.errors.len(), 1, "{c:?}");
        assert_eq!(
            c.warnings,
            vec![Issue {
                path: "/mcpServers/s/headersHelper".into(),
                message: "unknown field, ignored".into()
            }]
        );
    }

    #[test]
    fn a_shared_reference_is_listed_once_per_host() {
        let raw = json!({"url": "https://mcp.example/mcp", "headers": {"X-Key": "secret://S"},
            "auth": {"type": "client_credentials",
                "token_url": "https://api.linear.app/oauth/token", "client_id": "id",
                "client_secret": "secret://S"}});
        assert_eq!(
            check("s", raw).effects.sends,
            vec![
                send("secret://S", "mcp.example"),
                send("secret://S", "api.linear.app")
            ]
        );
    }

    #[test]
    fn issues_come_in_the_order_of_the_rules() {
        let c = check(
            "s",
            json!({"url": "http://example.com/mcp", "note": "${X}", "timeout": 1,
                "headers": {"Content-Type": "x", "X-Key": "abc"},
                "auth": {"type": "client_credentials", "audience": "x",
                    "token_url": "https://t.example/token?api_key=1"}}),
        );
        let issue = |path: &str, message: &str| Issue {
            path: format!("/mcpServers/s{path}"),
            message: message.into(),
        };
        assert_eq!(
            c.errors,
            vec![
                issue("/url", LOOPBACK_MSG),
                issue(
                    "/auth/token_url",
                    "query parameter `api_key` looks like a secret; make the whole value a reference (`secret://NAME`)"
                ),
                issue("/auth/client_secret", REF_ONLY),
                issue("/auth/client_id", "required for client_credentials"),
                issue(
                    "/headers/Content-Type",
                    "set by the transport, cannot be configured"
                ),
                issue("/headers/X-Key", NO_LITERAL),
                issue("/note", "use `env://VAR` instead of `${VAR}`"),
            ]
        );
        assert_eq!(
            c.warnings,
            vec![
                issue("/note", "unknown field, ignored"),
                issue("/timeout", "unknown field, ignored"),
                issue("/auth/audience", "unknown field, ignored"),
            ]
        );
    }
}
