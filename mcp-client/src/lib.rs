pub mod approvals;
pub mod config;
pub mod configuration;
pub mod functions;
pub mod host;
pub mod prompts;
pub mod reconcile;
pub mod refs;
pub mod resources;
pub mod server;
pub mod stdio;
pub mod translate;

/// The `III_MCP_CLIENT_STDIO` switch: only `off` (trimmed, any case) turns
/// stdio servers off.
pub fn stdio_allowed(value: Option<&str>) -> bool {
    !value.is_some_and(|v| v.trim().eq_ignore_ascii_case("off"))
}

/// Log targets whose level is pinned at INFO whatever `RUST_LOG` says: at
/// DEBUG and below they print authorization codes and token responses.
pub const CAPPED_LOG_TARGETS: [&str; 4] = ["rmcp", "reqwest", "hyper", "h2"];

/// Builds the `EnvFilter` directive string from `RUST_LOG`. A more specific
/// directive beats a capped one, and a span or field filter enables every
/// event inside the span whatever its target, so a user directive survives
/// only when it is plain `target=level` text for an uncapped target. Targets
/// match as string prefixes and the longer one wins, so `rmcp:=trace` would
/// beat `rmcp=info`: any target starting `<capped>:` is dropped too.
pub fn log_directives(rust_log: Option<&str>) -> String {
    let mut kept: Vec<String> = rust_log
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|d| {
            let plain = d
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '=' | '-'));
            let target = d.split('=').next().unwrap_or("");
            let capped = CAPPED_LOG_TARGETS
                .iter()
                .any(|t| target == *t || target.starts_with(&format!("{t}:")));
            !d.is_empty() && plain && !capped
        })
        .map(String::from)
        .collect();
    if kept.is_empty() {
        kept.push("info".into());
    }
    kept.extend(CAPPED_LOG_TARGETS.iter().map(|t| format!("{t}=info")));
    kept.join(",")
}

/// Turns off the SDK's handler input/output span events. Tool inputs and
/// results, authorize URLs and callback codes travel under keys the SDK's
/// key-name redaction does not mask, so this worker never records them.
///
/// # Safety
/// The entire process must be single-threaded.
pub unsafe fn disable_trace_payloads() {
    // SAFETY: the caller guarantees the process is single-threaded.
    unsafe { std::env::set_var("III_DISABLE_TRACE_PAYLOADS", "1") };
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::EnvFilter;

    const CAPS: &str = "rmcp=info,reqwest=info,hyper=info,h2=info";

    #[test]
    fn log_directives_cap_the_noisy_crates() {
        assert_eq!(log_directives(None), format!("info,{CAPS}"));
        assert_eq!(log_directives(Some("rmcp=trace")), format!("info,{CAPS}"));
        assert_eq!(
            log_directives(Some("trace,rmcp::transport::auth=trace")),
            format!("trace,{CAPS}")
        );
        assert_eq!(
            log_directives(Some(
                "debug, rmcp=trace,mcp_client=trace,h2,hyper::proto=debug"
            )),
            format!("debug,mcp_client=trace,{CAPS}")
        );
        // Another crate, not a module of a capped one.
        assert_eq!(
            log_directives(Some("hyper_util=debug")),
            format!("hyper_util=debug,{CAPS}")
        );
        assert_eq!(
            log_directives(Some("rmcp:=trace,h2:")),
            format!("info,{CAPS}")
        );
        assert_eq!(
            log_directives(Some("debug,reqwest:=trace,hyper::proto:=trace")),
            format!("debug,{CAPS}")
        );
        // Span and field filters go.
        assert_eq!(
            log_directives(Some("warn,[{server}]=trace,mcp_client[connect]=trace")),
            format!("warn,{CAPS}")
        );
    }

    #[test]
    fn stdio_is_on_unless_the_switch_says_off() {
        for on in [None, Some(""), Some("on")] {
            assert!(stdio_allowed(on), "{on:?}");
        }
        for off in ["off", " OFF "] {
            assert!(!stdio_allowed(Some(off)), "{off:?}");
        }
    }

    #[test]
    fn log_directives_always_parse() {
        for input in [
            None,
            Some("trace,rmcp=trace"),
            Some("debug,[x{a=\"b,c\"}]=trace"),
        ] {
            assert!(
                EnvFilter::try_new(log_directives(input)).is_ok(),
                "{input:?}"
            );
        }
    }
}
