use llm_router::types::credential::Credential;
use llm_router::types::router::ProviderResolveResponse;

pub const DEFAULT_API_URL: &str = "https://api.commandcode.ai/provider/v1";
pub const DEFAULT_MAX_TOKENS: u64 = 8192;

#[derive(Debug, Clone)]
pub struct CommandCodeConfig {
    pub credential_value: String,
    pub model: String,
    pub max_tokens: u64,
    pub base_url: String,
    pub zdr: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    NotConfigured,
    InvalidApiUrl(String),
    InvalidZdr(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => f.write_str(
                "provider command-code not configured (no api_key in the llm-router entry and COMMAND_CODE_API_KEY unset)",
            ),
            Self::InvalidApiUrl(value) => write!(
                f,
                "provider command-code has an invalid endpoint url: {value:?} (must be an absolute http(s) URL)",
            ),
            Self::InvalidZdr(value) => write!(
                f,
                "CMD_ZDR has invalid value {value:?}; use 1/true to require ZDR or 0/false to disable it",
            ),
        }
    }
}

pub fn credential_value(credential: &Credential) -> &str {
    match credential {
        Credential::ApiKey { key } => key,
        Credential::Oauth { access_token, .. } => access_token,
    }
}

pub fn normalize_base_url(value: &str) -> Result<String, ConfigError> {
    let value = value.trim().trim_end_matches('/');
    let base = ["/chat/completions", "/messages", "/models"]
        .iter()
        .find_map(|suffix| value.strip_suffix(suffix))
        .unwrap_or(value)
        .trim_end_matches('/');
    let parsed =
        reqwest::Url::parse(base).map_err(|_| ConfigError::InvalidApiUrl(value.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ConfigError::InvalidApiUrl(value.to_string()));
    }
    Ok(base.to_string())
}

pub fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}/{path}", base_url.trim_end_matches('/'))
}

pub fn parse_zdr(value: Option<&str>) -> Result<bool, ConfigError> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("0" | "false" | "FALSE" | "False") => Ok(false),
        Some("1" | "true" | "TRUE" | "True") => Ok(true),
        Some(value) => Err(ConfigError::InvalidZdr(value.to_string())),
    }
}

pub fn config_from_resolve(
    model: &str,
    effective_max_tokens: Option<u64>,
    resolved: &ProviderResolveResponse,
) -> Result<CommandCodeConfig, ConfigError> {
    let credential = resolved
        .credential
        .as_ref()
        .map(credential_value)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::NotConfigured)?;
    let base_url = normalize_base_url(
        resolved
            .api_url
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(DEFAULT_API_URL),
    )?;
    let zdr = parse_zdr(std::env::var("CMD_ZDR").ok().as_deref())?;
    Ok(CommandCodeConfig {
        credential_value: credential.to_string(),
        model: model.to_string(),
        max_tokens: effective_max_tokens
            .or(resolved.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS),
        base_url,
        zdr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::router::CredentialSource;

    fn resolved(api_url: Option<&str>, max_tokens: Option<u64>) -> ProviderResolveResponse {
        ProviderResolveResponse {
            configured: true,
            source: CredentialSource::Config,
            credential: Some(Credential::ApiKey {
                key: "secret\n".into(),
            }),
            api_url: api_url.map(str::to_string),
            max_tokens,
        }
    }

    #[test]
    fn endpoint_overrides_normalize_to_one_base() {
        for value in [
            "https://example.test/provider/v1",
            "https://example.test/provider/v1/chat/completions",
            "https://example.test/provider/v1/messages/",
            "https://example.test/provider/v1/models",
        ] {
            assert_eq!(
                normalize_base_url(value).unwrap(),
                "https://example.test/provider/v1"
            );
        }
    }

    #[test]
    fn invalid_urls_are_rejected_before_reqwest() {
        assert!(matches!(
            normalize_base_url("localhost:8080"),
            Err(ConfigError::InvalidApiUrl(_))
        ));
        assert!(matches!(
            normalize_base_url("file:///tmp/provider"),
            Err(ConfigError::InvalidApiUrl(_))
        ));
    }

    #[test]
    fn zdr_parser_is_explicit_and_fail_closed_on_typos() {
        assert!(!parse_zdr(None).unwrap());
        assert!(parse_zdr(Some("1")).unwrap());
        assert!(parse_zdr(Some("true")).unwrap());
        assert!(!parse_zdr(Some("false")).unwrap());
        assert_eq!(
            parse_zdr(Some("enabled")).unwrap_err(),
            ConfigError::InvalidZdr("enabled".into())
        );
    }

    #[test]
    fn max_tokens_precedence_and_credential_trimming() {
        let cfg = config_from_resolve("m", Some(1024), &resolved(None, Some(2048))).unwrap();
        assert_eq!(cfg.max_tokens, 1024);
        assert_eq!(cfg.credential_value, "secret");
        assert_eq!(cfg.base_url, DEFAULT_API_URL);
    }
}
