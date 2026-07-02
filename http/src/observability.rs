//! Tracing -> OpenTelemetry bridge wiring for the `iii-http` worker.
//!
//! `main` builds the worker's `tracing_subscriber` as a registry with both the
//! `fmt` layer (human logs) and the [`otel_layer`] bridge, so the per-request
//! `HTTP` spans emitted in [`crate::handler`] reach the global OTel SDK and
//! `Span::set_parent` resolves instead of returning
//! `SetParentError::LayerNotFound`.
//!
//! ## Why a lazy tracer
//!
//! The SDK's `init_otel` installs the global `TracerProvider` asynchronously on
//! a background connection thread (spawned inside `register_worker`), *after*
//! `main` installs the subscriber. `opentelemetry::global::tracer(name)`
//! **snapshots** the provider that is current when it is called (opentelemetry
//! 0.31: `global::tracer` clones the current global provider, defaulting to the
//! no-op provider until `set_tracer_provider` runs). Binding the layer to
//! `global::tracer(...)` at subscriber-install time would therefore capture the
//! no-op provider and never export.
//!
//! [`LazyGlobalTracer`] instead re-reads `global::tracer` on every span build,
//! so spans created per request (long after the provider is installed) resolve
//! to the real exporter. This is the same pattern the SDK itself uses for its
//! invocation spans (`global::tracer("iii-rust-sdk")` per call), and it lets the
//! subscriber install early -- preserving startup log output.

use iii_helpers::observability::opentelemetry::Context;
use iii_helpers::observability::opentelemetry::global::{self, BoxedSpan};
use iii_helpers::observability::opentelemetry::trace::{SpanBuilder, Tracer};
use tracing::Subscriber;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

/// Instrumentation scope name for the worker's tracer.
const TRACER_NAME: &str = "iii-http";

/// A `Tracer` that delegates to the current global provider on every span build.
///
/// Unlike a tracer obtained once from `global::tracer`, this re-reads the global
/// `TracerProvider` per span, so spans export correctly even though the provider
/// is installed after the subscriber (see module docs).
#[derive(Debug, Clone, Default)]
pub struct LazyGlobalTracer;

impl Tracer for LazyGlobalTracer {
    type Span = BoxedSpan;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> Self::Span {
        global::tracer(TRACER_NAME).build_with_context(builder, parent_cx)
    }
}

/// The `tracing` -> OpenTelemetry bridge layer for the worker subscriber.
///
/// Installing this makes `Span::set_parent` resolve (the layer registers the
/// `WithContext` extension the downcast looks for) and bridges emitted spans to
/// whatever global OTel SDK provider is current at span-build time.
pub fn otel_layer<S>() -> OpenTelemetryLayer<S, LazyGlobalTracer>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    tracing_opentelemetry::layer().with_tracer(LazyGlobalTracer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    use tracing_subscriber::prelude::*;

    /// Under a subscriber carrying [`otel_layer`], `Span::set_parent` must
    /// resolve (return `Ok`) instead of `SetParentError::LayerNotFound`. This is
    /// the exact failure the production subscriber had before the bridge was
    /// wired: with only a `fmt` subscriber, the `WithContext` downcast fails and
    /// every traceparent request loses its parent. A real provider is not needed
    /// here -- `set_parent` succeeds whenever *any* OTel layer is present.
    #[test]
    fn otel_layer_makes_set_parent_resolve() {
        let subscriber = tracing_subscriber::registry().with(otel_layer());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("HTTP");
            let cx = iii_helpers::observability::extract_context(
                Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"),
                None,
            );
            assert!(
                span.set_parent(cx).is_ok(),
                "otel bridge layer must be present so set_parent resolves (not LayerNotFound)"
            );
        });
    }

    /// Sanity guard for the inverse: with **no** OTel layer (a bare registry),
    /// `set_parent` returns `LayerNotFound` -- documenting exactly what the
    /// bridge fixes.
    #[test]
    fn without_otel_layer_set_parent_is_layer_not_found() {
        let subscriber = tracing_subscriber::registry();
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("HTTP");
            let cx = iii_helpers::observability::extract_context(
                Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"),
                None,
            );
            assert!(
                span.set_parent(cx).is_err(),
                "without an otel layer set_parent must fail (LayerNotFound)"
            );
        });
    }
}
