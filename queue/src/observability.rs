//! Tracing-to-OpenTelemetry bridge for queue delivery spans.
//!
//! The SDK installs the global provider after the process subscriber starts,
//! so the layer uses a tracer that resolves the current provider for every
//! span instead of capturing the startup no-op provider.

use iii_helpers::observability::opentelemetry::global::{self, BoxedSpan};
use iii_helpers::observability::opentelemetry::trace::{SpanBuilder, Tracer};
use iii_helpers::observability::opentelemetry::Context;
use tracing::Subscriber;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

const TRACER_NAME: &str = "iii-queue";

#[derive(Debug, Clone, Default)]
pub struct LazyGlobalTracer;

impl Tracer for LazyGlobalTracer {
    type Span = BoxedSpan;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> Self::Span {
        global::tracer(TRACER_NAME).build_with_context(builder, parent_cx)
    }
}

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

    #[test]
    fn otel_layer_allows_queue_spans_to_attach_a_parent() {
        let subscriber = tracing_subscriber::registry().with(otel_layer());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("function_queue_job");
            let parent = iii_helpers::observability::extract_context(
                Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"),
                None,
            );
            assert!(span.set_parent(parent).is_ok());
        });
    }
}
