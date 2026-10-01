use tracing_subscriber::{
    EnvFilter, Layer, filter::filter_fn, layer::SubscriberExt, util::SubscriberInitExt,
};

/// Native protocol logging can contain pairing codes and decrypted payloads.
/// Keep diagnostic output restricted to this worker, even under RUST_LOG=trace.
fn worker_metadata(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.target() == "whatsapp" || metadata.target().starts_with("whatsapp::")
}

pub fn init() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(filter_fn(worker_metadata)),
        )
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn verbose_logging_cannot_expose_upstream_pairing_or_protocol_payloads() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = Capture(output.clone());
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new("trace"))
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_ansi(false)
                    .with_writer(move || writer.clone())
                    .with_filter(filter_fn(worker_metadata)),
            );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "Client/PairCode", "PAIR-SECRET");
            tracing::debug!(target: "whatsapp_rust::client", "MESSAGE-SECRET");
            tracing::info!(target: "whatsapp::sessions", "worker diagnostic");
        });
        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(!output.contains("SECRET"), "{output}");
        assert!(output.contains("worker diagnostic"), "{output}");
    }
}
