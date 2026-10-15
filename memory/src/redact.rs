//! Deterministic secret redaction for everything the worker persists or
//! hands to a model: memory text and entities, rules (always injected), and
//! the transcript excerpt extraction sends to `router::complete`. The
//! extraction prompt's "never store secrets" is a request; this is the
//! guarantee for the shapes it knows.
//!
//! A denylist of shapes, never complete: a secret in an unknown format
//! passes. Secrets only — no PII (an email or a phone number is a
//! legitimate memory). Structural shapes come from the sentinel worker's
//! redactor (whole PEM blocks, URL credentials, bearer tokens, key=value
//! assignments); prefixed provider keys from Hindsight's Memory Defense
//! catalog (`extensions/memory_defense.py`), in its order: specific prefixes
//! before generic ones. A match becomes `[REDACTED:<label>]`.

use std::sync::LazyLock;

use regex::Regex;

/// The redacted text and the label of every shape that matched (deduped,
/// in first-match order). Never carries the secret itself.
#[derive(Debug, Default, PartialEq)]
pub struct Redaction {
    pub text: String,
    pub labels: Vec<&'static str>,
}

struct Rule {
    label: &'static str,
    pattern: Regex,
    /// Capture group to replace (0 = the whole match), so a URL keeps its
    /// scheme and host and an assignment keeps its key.
    group: usize,
    /// The match must not touch an ASCII word character on either side
    /// (Rust's regex has no lookaround; `\b` also fails beside CJK text).
    token: bool,
    /// Extra test on the replaced text, against ordinary prose.
    accept: fn(&str) -> bool,
}

fn any(_: &str) -> bool {
    true
}

/// Values that NAME a secret instead of holding one: an env or config
/// reference (`$VAR`, `<your-key>`, `OPENAI_API_KEY`, `config.api_key`,
/// `process.env.X`), a code path or type (`Token::String`, `Option<String>`,
/// `Token::new()`), or a location (a path; `secret://NAME`, `env://NAME`, a
/// URL — a credentialed URL is url_credentials' job). Where a secret lives
/// is a fact worth keeping.
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:",
        r"\$[A-Za-z_][A-Za-z0-9_]*",       // $VAR
        r"|<[^>]*>",                       // <your-key>
        r"|[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+", // OPENAI_API_KEY
        r"|[A-Za-z_][A-Za-z0-9_]*(?:(?:\.|::)[A-Za-z_][A-Za-z0-9_]*)+(?:\(\))?", // a.b, A::b()
        r"|[A-Za-z_][A-Za-z0-9_:]*<[^>]*>", // Option<String>
        r"|[A-Za-z_][A-Za-z0-9_:]*\(\)",   // new()
        r"|~?\.{0,2}/[^+=]*",              // /path, ~/x, ./x
        r"|[A-Za-z][A-Za-z0-9+.\-]*://.*", // scheme://...
        r")$",
    ))
    .expect("reference pattern compiles")
});

/// An assigned value that looks like a credential rather than a word, a
/// number, or a reference: not a [`REFERENCE`], not all digits
/// (`max_token=128000`), and a digit or a symbol in it. `_ - . /` are not
/// symbols (identifiers, kebab names, paths); a letters-only passphrase
/// passes, the price of keeping type and resource names.
fn credential_like(value: &str) -> bool {
    // A leading `:` means the key was a `token::`/`secret::` code path, not
    // an assignment.
    !value.starts_with(':')
        && !REFERENCE.is_match(value)
        && !value.bytes().all(|b| b.is_ascii_digit())
        && value
            .bytes()
            .any(|b| b.is_ascii_digit() || b"!@#%^*+=~$&<>?".contains(&b))
}

/// A bearer token carries a digit; "Bearer authentication-middleware" does
/// not.
fn has_digit(value: &str) -> bool {
    value.bytes().any(|b| b.is_ascii_digit())
}

fn rule(
    label: &'static str,
    pattern: &str,
    group: usize,
    token: bool,
    accept: fn(&str) -> bool,
) -> Rule {
    Rule {
        label,
        pattern: Regex::new(pattern).expect("built-in redaction patterns compile"),
        group,
        token,
        accept,
    }
}

/// Prefix-anchored provider key: replaced whole, word-bounded.
fn key(label: &'static str, pattern: &str) -> Rule {
    rule(label, pattern, 0, true, any)
}

/// Order matters: structural shapes first, so a PEM block or a credentialed
/// URL is taken whole before a key rule nibbles at it; specific prefixes
/// before the generic `sk-`.
static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(
            "private_key",
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY( BLOCK)?-----.*?-----END [A-Z ]*PRIVATE KEY( BLOCK)?-----",
            0,
            false,
            any,
        ),
        // A truncated block (the 2000-char cap, a partial paste) has no END:
        // the header and the base64 body that follows it (after optional
        // `Proc-Type:`-style lines). A bare mention of the header is prose.
        rule(
            "private_key",
            r"-----BEGIN [A-Z ]*PRIVATE KEY( BLOCK)?-----\s*(?:[A-Za-z-]+:[^\n]*\n\s*)*[A-Za-z0-9+/=]{40}[A-Za-z0-9+/=\s]*",
            0,
            false,
            any,
        ),
        // The password runs to the LAST `@` before the path, query, or
        // fragment: an unescaped `@` in it (`u:p@ss@host`) must not leave
        // the rest visible.
        rule(
            "url_credentials",
            r"(?i)(?-u:\b)[a-z][a-z0-9+.\-]*://([^:/@\s]*:[^\s/?#]+)@",
            1,
            false,
            any,
        ),
        rule(
            "bearer_token",
            r"(?i)(?-u:\b)bearer\s+([A-Za-z0-9\-._~+/]{20,}=*)",
            1,
            false,
            has_digit,
        ),
        key(
            "jwt",
            r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
        ),
        rule(
            "aws_secret_key",
            r#"(?i)aws.{0,20}?(?:secret|private)?[\s_-]?access[\s_-]?key[\s_-]?[:=][\s"']*([A-Za-z0-9/+=]{40})"#,
            1,
            false,
            any,
        ),
        key("anthropic_key", r"sk-ant-[A-Za-z0-9_-]{20,}"),
        key(
            "openai_key",
            r"sk-(?:proj|svcacct|admin)-[A-Za-z0-9_-]{20,}",
        ),
        // The generic form has no hyphens: kebab words never match.
        key("openai_key", r"sk-[A-Za-z0-9]{32,}"),
        key("google_api_key", r"AIza[0-9A-Za-z_-]{35}"),
        key("google_oauth_token", r"ya29\.[0-9A-Za-z_-]{20,}"),
        key("xai_key", r"xai-[A-Za-z0-9]{40,}"),
        key("groq_key", r"gsk_[A-Za-z0-9]{20,}"),
        key("huggingface_token", r"hf_[A-Za-z0-9]{30,}"),
        key("replicate_token", r"r8_[A-Za-z0-9]{30,}"),
        key("perplexity_key", r"pplx-[A-Za-z0-9]{40,}"),
        key("databricks_token", r"dapi[A-Za-z0-9]{32}"),
        key("aws_access_key", r"(?:AKIA|ASIA)[0-9A-Z]{16}"),
        key("digitalocean_token", r"dop_v1_[a-f0-9]{64}"),
        key("github_token", r"github_pat_[A-Za-z0-9_]{60,}"),
        key("github_token", r"gh[pousr]_[A-Za-z0-9]{36}"),
        key("gitlab_token", r"glpat-[A-Za-z0-9_-]{20,}"),
        key("npm_token", r"npm_[A-Za-z0-9]{30,}"),
        key("pypi_token", r"pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{20,}"),
        key("stripe_key", r"[sr]k_(?:live|test)_[A-Za-z0-9]{20,}"),
        key("square_token", r"sq0[a-z]{3}-[A-Za-z0-9_-]{22,}"),
        key("slack_token", r"xox[abprs]-[0-9A-Za-z-]{10,}"),
        rule(
            "slack_webhook",
            r"https://hooks\.slack\.com/services/T[A-Za-z0-9_]{8,}/B[A-Za-z0-9_]{8,}/[A-Za-z0-9_]{20,}",
            0,
            false,
            any,
        ),
        key("sendgrid_key", r"SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}"),
        key(
            "discord_token",
            r"[MNO][A-Za-z0-9_-]{23,25}\.[A-Za-z0-9_-]{6}\.[A-Za-z0-9_-]{27,38}",
        ),
        key("telegram_token", r"[0-9]{8,10}:[A-Za-z0-9_-]{35}"),
        // In its API-URL form `/bot<id>:<token>` the `bot` prefix touches it.
        rule(
            "telegram_token",
            r"/bot([0-9]{8,10}:[A-Za-z0-9_-]{35})",
            1,
            false,
            any,
        ),
        key("shopify_token", r"shpat_[a-fA-F0-9]{32}"),
        // `api_key: …`, `password=…`, `"token": "…"`: the key stays, the
        // value goes — when the value looks like a credential.
        rule(
            "credential",
            // `(?:\b|_)`: `\b` never matches after `_` (`DB_PASSWORD=`), and
            // `password` needs no left edge at all (`PGPASSWORD=`). ASCII
            // `\b`: Unicode sees no edge between CJK and ASCII. The value
            // may hold `$ & < >` (common in passwords); `[ ] { } \`` end it,
            // so `${VAR}` and a placeholder never match, and a trailing `.`
            // stays with the sentence. References are `credential_like`'s.
            r#"(?i)(?:(?:(?-u:\b)|_)(?:api[_-]?key|access[_-]?key|secret[_-]?key|client[_-]?secret|private[_-]?key|auth[_-]?token|access[_-]?token|refresh[_-]?token|token|secret)|password|passwd)(?-u:\b)"?\s*[:=]\s*["']?([^\s,;"'\[\]{}`]{5,}[^\s,;"'\[\]{}`.])"#,
            1,
            false,
            credential_like,
        ),
        // Prose: "my password is hunter2!".
        rule(
            "credential",
            r#"(?i)(?-u:\b)(?:password|passcode|passphrase)\s+(?:is|was)\s+["']?([^\s,;"'\[\]{}`]{5,}[^\s,;"'\[\]{}`.])"#,
            1,
            false,
            credential_like,
        ),
    ]
});

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Replace every known secret shape in `text` with `[REDACTED:<label>]`.
pub fn redact(text: &str) -> Redaction {
    let mut current = text.to_string();
    let mut labels: Vec<&'static str> = Vec::new();
    for rule in RULES.iter() {
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for caps in rule.pattern.captures_iter(&current) {
            let (Some(whole), Some(target)) = (caps.get(0), caps.get(rule.group)) else {
                continue;
            };
            if rule.token
                && (is_word(current[..whole.start()].chars().next_back())
                    || is_word(current[whole.end()..].chars().next()))
            {
                continue;
            }
            // Already a placeholder (a re-save, the commit backstop after an
            // entry point): no new redaction, so no label either.
            if target.as_str().starts_with("[REDACTED:") || !(rule.accept)(target.as_str()) {
                continue;
            }
            spans.push((target.start(), target.end()));
        }
        if spans.is_empty() {
            continue;
        }
        let placeholder = format!("[REDACTED:{}]", rule.label);
        for (start, end) in spans.into_iter().rev() {
            current.replace_range(start..end, &placeholder);
        }
        if !labels.contains(&rule.label) {
            labels.push(rule.label);
        }
    }
    Redaction {
        text: current,
        labels,
    }
}

/// Drop the short fields (entities, tags) that carry a secret shape; the
/// labels hit. Dropped, not redacted: a `[REDACTED:x]` entity would get the
/// entity boost in recall and show up in the tag list.
pub fn drop_secrets(fields: &mut Vec<String>) -> Vec<&'static str> {
    let mut labels = Vec::new();
    fields.retain(|field| {
        let hit = redact(field).labels;
        for label in &hit {
            if !labels.contains(label) {
                labels.push(*label);
            }
        }
        hit.is_empty()
    });
    labels
}

/// `redacted: <labels>`, the reason an `item-changed` event carries.
pub fn reason(labels: &[&'static str]) -> Option<String> {
    (!labels.is_empty()).then(|| format!("redacted: {}", labels.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One realistic-shape sample per rule (none of them live credentials).
    fn samples() -> Vec<(&'static str, String)> {
        let a = |n: usize| "a1B2c3D4e5".repeat(n / 10 + 1)[..n].to_string();
        let hex = |n: usize| "0123456789abcdef".repeat(n / 16 + 1)[..n].to_string();
        // Built at runtime: a PEM header is the same text in every key, so
        // the repo's push guard cannot allowlist it without going blind to
        // real ones.
        let pem = |kind: &str, edge: &str| format!("-----{edge} {kind}PRIVATE KEY-----");
        // Credentialed URLs too, scheme apart: same reason.
        let url = |scheme: &str, rest: &str| format!("{scheme}://{rest}");
        vec![
            (
                "private_key",
                format!(
                    "{}\nMIIEowIBAAKCAQEA7\nabc\n{}",
                    pem("RSA ", "BEGIN"),
                    pem("RSA ", "END")
                ),
            ),
            (
                "private_key",
                format!(
                    "{}\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB truncated",
                    pem("OPENSSH ", "BEGIN")
                ),
            ),
            (
                "url_credentials",
                url("postgres", "admin:S3cr3tPass@db.internal:5432/app"),
            ),
            ("url_credentials", "redis://:hunter2pw@cache:6379".into()),
            ("bearer_token", format!("Authorization: Bearer {}", a(32))),
            ("jwt", format!("eyJ{}.eyJ{}.{}", a(16), a(20), a(24))),
            (
                "aws_secret_key",
                format!("aws_secret_access_key = {}", a(40)),
            ),
            ("anthropic_key", format!("sk-ant-api03-{}", a(40))),
            ("openai_key", format!("sk-proj-{}", a(60))),
            ("openai_key", format!("sk-svcacct-{}", a(44))),
            ("openai_key", format!("sk-{}", a(48))),
            ("google_api_key", format!("AIza{}", a(35))),
            ("google_oauth_token", format!("ya29.{}", a(30))),
            ("xai_key", format!("xai-{}", a(44))),
            ("groq_key", format!("gsk_{}", a(40))),
            ("huggingface_token", format!("hf_{}", a(34))),
            ("replicate_token", format!("r8_{}", a(34))),
            ("perplexity_key", format!("pplx-{}", a(44))),
            ("databricks_token", format!("dapi{}", a(32))),
            ("aws_access_key", "AKIAIOSFODNN7EXAMPLE".into()),
            ("aws_access_key", "ASIAIOSFODNN7EXAMPLE".into()),
            ("digitalocean_token", format!("dop_v1_{}", hex(64))),
            ("github_token", format!("github_pat_{}", a(70))),
            ("github_token", format!("ghp_{}", a(36))),
            ("github_token", format!("gho_{}", a(36))),
            ("gitlab_token", format!("glpat-{}", a(20))),
            ("npm_token", format!("npm_{}", a(36))),
            ("pypi_token", format!("pypi-AgEIcHlwaS5vcmc{}", a(40))),
            ("stripe_key", format!("sk_live_{}", a(24))),
            ("stripe_key", format!("rk_test_{}", a(24))),
            ("square_token", format!("sq0atp-{}", a(22))),
            ("slack_token", format!("xoxb-{}", a(24))),
            (
                "slack_webhook",
                format!(
                    "https://hooks.slack.com/services/T{}/B{}/{}",
                    a(9),
                    a(9),
                    a(24)
                ),
            ),
            ("sendgrid_key", format!("SG.{}.{}", a(22), a(43))),
            ("discord_token", format!("M{}.{}.{}", a(23), a(6), a(27))),
            ("discord_token", format!("M{}.{}.{}", a(25), a(6), a(38))),
            ("telegram_token", format!("123456789:{}", a(35))),
            (
                "telegram_token",
                format!("https://api.telegram.org/bot123456789:{}/getMe", a(35)),
            ),
            ("shopify_token", format!("shpat_{}", hex(32))),
            ("credential", "api_key=Zx81!kLm0Pq".into()),
            ("credential", r#""password": "hunter2!x""#.into()),
            ("credential", "my password is hunter2!".into()),
            ("credential", "MY_SERVICE_API_KEY=abc123xyz789qq".into()),
            ("credential", "export DB_PASSWORD=Tr0ub4dor&3xyz".into()),
            ("credential", "DB password=Pa$$w0rd2024".into()),
            ("credential", r#""api_key": "AbC$1234567890xyzQ""#.into()),
            ("credential", "PGPASSWORD=Sup3rS3cret!".into()),
            ("credential", "密码password=hunter2!x".into()),
            (
                "url_credentials",
                format!("数据库{}", url("postgres", "u:Pw9xYz77@db/app")),
            ),
            ("url_credentials", url("amqp", "guest:Gu3stPw9@mq:5672/vh")),
        ]
    }

    #[test]
    fn every_known_shape_is_redacted_and_never_survives() {
        for (label, secret) in samples() {
            let text = format!("User noted: {secret} — keep it handy.");
            let r = redact(&text);
            assert!(
                r.labels.contains(&label),
                "{label}: {:?} -> {}",
                r.labels,
                r.text
            );
            assert!(
                r.text.contains(&format!("[REDACTED:{label}]")),
                "{label}: {}",
                r.text
            );
            // The secret part never survives (a URL keeps its host on
            // purpose, so its password is what must be gone).
            let core = match secret.as_str() {
                s if s.contains("S3cr3tPass") => "S3cr3tPass",
                s if s.contains("hunter2pw") => "hunter2pw",
                s if s.contains("Gu3stPw9") => "Gu3stPw9",
                s if s.contains("Tr0ub4dor") => "&3xyz",
                s if s.contains("Pw9xYz77") => "Pw9xYz77",
                s => &s[s.len().saturating_sub(10)..],
            };
            assert!(
                !r.text.contains(core),
                "{label} leaked {core:?}: {}",
                r.text
            );
        }
    }

    #[test]
    fn structure_around_a_secret_is_kept() {
        let pg = |rest: &str| format!("{}://{rest}", "postgres");
        assert_eq!(
            redact(&pg("admin:S3cr3tPass@db:5432/app")).text,
            pg("[REDACTED:url_credentials]@db:5432/app")
        );
        // An unescaped `@` in the password: all of it goes, the host stays.
        assert_eq!(
            redact(&pg("admin:p4ss@w0rd@db:5432/app?ssl=1")).text,
            pg("[REDACTED:url_credentials]@db:5432/app?ssl=1")
        );
        assert_eq!(
            redact("api_key: Zx81!kLm0Pq").text,
            "api_key: [REDACTED:credential]"
        );
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        let benign = [
            "User prefers dark mode and short answers.",
            "Deployed commit 3f2a9c1d4e5b6a7c8d9e0f1a2b3c4d5e6f7a8b9c to production.",
            "Session id 550e8400-e29b-41d4-a716-446655440000 crashed.",
            "Read sk-learning-resources-and-tutorials before the workshop.",
            "He was the bearer of bad news.",
            "See https://example.com/docs?page=2 and postgres://localhost:5432/app.",
            "Upgraded from v1.2.3 to v1.10.0 on 2026-10-14.",
            "User's work email is mike@example.com and phone +1 555 0100.",
            "The password is required to log in.",
            "token: gold, secret: none",
            "sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "Meeting at 10:30, room B-12, budget $4,500.",
            "pwd: /home/user/projects",
            "The password is incorrect.",
            "Here's the secret: re-reading the docs helps.",
            "access_token=${ACCESS_TOKEN}",
            "api_key: <your-key-here>",
            "token: 200000 and max_token=128000",
            "pnpm_lockfile_version_9 is pinned",
            "sk-learn-2024-tutorial-series is a course",
            "ssh://git@github.com/org/repo and https://user@host/path",
            "Bearer tokens are sent in the Authorization header.",
            "ULID 01ARZ3NDEKTSV4RRFFQ69G5FAV, base64 id dGhpcyBpcyBhbiBpZA==",
            "Released v0.24.5-rc.1.",
            "TLS secret: /etc/ssl/private/ledgerly.key",
            "GitHub CLI stores its token: ~/.config/gh/hosts.yml",
            "token: /var/run/secrets/kubernetes.io/serviceaccount/token is mounted",
            "Deploys use private_key: ~/.ssh/id_ed25519",
            "Vault secret: https://vault.internal:8200/v1/kv/data/ledgerly",
            "The judge provider uses api_key: secret://TYPESAFE_API_KEY.",
            "Configure token = env://GITHUB_TOKEN for CI",
            "apiKey: process.env.OPENAI_API_KEY",
            "The webhook secret: WHATSAPP_META_APP_SECRET env var",
            "let token = Token::new();",
            "token: Option<String> in the struct",
            "fields token: CancellationToken and secret: SecretString",
            "GITHUB_TOKEN=your_access_token_here",
            "Helm values set secret: postgres-credentials",
            "Never commit files containing the PRIVATE KEY header; rotate on leak.",
            "Bearer authentication-middleware guards the route.",
            "Call token::utf8_decode and password::hash_password2 first.",
        ];
        for text in benign {
            let r = redact(text);
            assert_eq!(r.text, text, "false positive {:?}", r.labels);
            assert!(r.labels.is_empty());
        }
        // A `key-<32 hex>` id (Mailgun's old key shape, which GitHub's push
        // protection flags as a literal) is an identifier here.
        let id = format!("key-{} is an id", "0123456789abcdef".repeat(2));
        assert_eq!(redact(&id).text, id);
        // A mention of a full PEM header (no key body) is prose: it must not
        // erase the instruction around it.
        let header = format!("-----BEGIN {}PRIVATE KEY-----", "RSA ");
        let mention = format!("Never commit files containing {header}; use the secrets worker.");
        assert_eq!(redact(&mention).text, mention);
    }

    #[test]
    fn a_key_inside_a_longer_word_is_not_a_key() {
        let partial = format!("Xghp_{}", &"a1B2c3D4e5".repeat(4)[..36]);
        assert!(redact(&partial).labels.is_empty());
    }

    #[test]
    fn a_key_beside_cjk_text_is_still_found() {
        let r = redact(&format!("密钥ghp_{}中文", &"a1B2c3D4e5".repeat(4)[..36]));
        assert_eq!(r.text, "密钥[REDACTED:github_token]中文");
    }

    #[test]
    fn redaction_is_idempotent() {
        for (label, secret) in samples() {
            let once = redact(&secret).text;
            let twice = redact(&once);
            assert_eq!(twice.text, once);
            // No label on a re-run either: the commit backstop would warn
            // and save would report a redaction that never happened.
            assert!(
                twice.labels.is_empty(),
                "{label} re-matched: {:?}",
                twice.labels
            );
        }
    }

    #[test]
    fn short_fields_and_the_event_reason() {
        let mut fields = vec!["alice".to_string(), "AKIAIOSFODNN7EXAMPLE".to_string()];
        let labels = drop_secrets(&mut fields);
        assert_eq!(fields, ["alice"]);
        assert_eq!(reason(&labels).as_deref(), Some("redacted: aws_access_key"));
        assert_eq!(reason(&[]), None);
    }
}
