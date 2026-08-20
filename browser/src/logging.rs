//! Log-filter construction for the worker binary.
//!
//! chromiumoxide 0.9.1's protocol bindings lag the system Chromium, so events
//! carrying enum values added since (e.g. DOM.pseudoElementAdded with
//! `overscroll-backdrop` on Chrome 151) fail its untagged-enum deserialize and
//! its handler WARN-spams "WS Invalid message" on every such frame — dozens
//! per page load. The frames are dropped either way (`ignore_invalid_messages`
//! defaults on; command responses can't fail this way, only events), so the
//! WARN carries no signal an operator can act on. Demote that module to
//! `error` unless the operator's RUST_LOG addresses chromiumoxide explicitly.
//!
//! ponytail: known ceiling — a Network.loadingFailed carrying one of the
//! Chrome-151 corsError values is still silently dropped before our listener;
//! recovering it needs chromiumoxide regenerated against the newer protocol
//! (no such release yet; 0.9.1 is current).

use tracing_subscriber::EnvFilter;

/// The worker's env filter: RUST_LOG (default `info`), with
/// `chromiumoxide::handler` demoted to `error` unless RUST_LOG mentions
/// chromiumoxide — an explicit operator directive always wins.
pub fn env_filter(rust_log: Option<&str>) -> EnvFilter {
    let mut filter = match rust_log {
        Some(directives) => EnvFilter::new(directives),
        None => EnvFilter::new("info"),
    };
    if rust_log.is_none_or(|directives| !directives.contains("chromiumoxide")) {
        filter = filter.add_directive(
            "chromiumoxide::handler=error"
                .parse()
                .expect("static directive parses"),
        );
    }
    filter
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn captured(filter: EnvFilter) -> String {
        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::registry().with(filter).with(
            tracing_subscriber::fmt::layer()
                .with_writer(move || writer.clone())
                .with_ansi(false),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: "chromiumoxide::handler", "WS Invalid message");
            tracing::error!(target: "chromiumoxide::handler", "WS Connection error");
            tracing::warn!(target: "browser::session", "worker warn passes");
        });
        let bytes = buffer.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn default_filter_drops_the_invalid_message_spam_but_keeps_errors() {
        let out = captured(env_filter(None));
        assert!(
            !out.contains("WS Invalid message"),
            "spam not dropped:\n{out}"
        );
        assert!(
            out.contains("WS Connection error"),
            "real errors lost:\n{out}"
        );
        assert!(
            out.contains("worker warn passes"),
            "worker warns lost:\n{out}"
        );
    }

    #[test]
    fn explicit_chromiumoxide_directive_in_rust_log_wins() {
        let out = captured(env_filter(Some("info,chromiumoxide=warn")));
        assert!(
            out.contains("WS Invalid message"),
            "operator's explicit directive was overridden:\n{out}"
        );
    }
}
