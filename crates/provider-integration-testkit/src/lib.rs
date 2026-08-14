//! Hermetic provider contract suite.
//!
//! The production engine, router, and provider code run unchanged. Only the
//! vendor HTTP boundary is replaced with a loopback server. Secrets in this
//! crate are fixed dummy values and captured requests are redacted before
//! rendering diagnostics.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

mod case;
mod contract;
mod protocol;
mod runtime;
mod stub;

pub use case::{ProtocolFamily, ANTHROPIC_MESSAGES, OPENAI_CHAT_COMPLETIONS, OPENAI_RESPONSES};
pub use stub::CapturedRequest;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::enabled_cases;
    use crate::contract::run_contract;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires III_ENGINE_BIN; executed by the provider-contract CI job"]
    async fn provider_contract() {
        let cases = enabled_cases();
        assert!(
            !cases.is_empty(),
            "enable exactly one provider-* feature when running this contract"
        );
        for case in cases {
            if let Err(error) = run_contract(case).await {
                panic!("{} contract failed: {error:#}", case.id);
            }
        }
    }

    #[test]
    fn rendered_requests_are_redacted() {
        let request = CapturedRequest {
            method: "POST".into(),
            path: "/v1/responses".into(),
            headers: vec![
                ("authorization".into(), "Bearer secret".into()),
                ("x-api-key".into(), "secret".into()),
                ("content-type".into(), "application/json".into()),
            ],
            body: "{}".into(),
        };
        let redacted = serde_json::to_string(&request.redacted()).unwrap();
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn every_enabled_provider_has_one_or_more_cases() {
        #[allow(unused_mut)]
        let mut expected = 0;
        #[cfg(feature = "provider-anthropic")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-claude-code")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-deepseek")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-kimi")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-openai")]
        {
            expected += 2;
        }
        #[cfg(feature = "provider-openai-codex")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-openrouter")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-xai")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-zai")]
        {
            expected += 1;
        }
        assert_eq!(enabled_cases().len(), expected);
    }
}
