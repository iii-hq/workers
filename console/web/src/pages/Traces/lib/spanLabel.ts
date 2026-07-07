import type { VisualizationSpan } from './traceTransform'

// Engine routing spans are reliably identified by their verb prefix —
// `handle_invocation X` (the engine's dispatch wrapper) and `call X` (the
// child span that actually invokes the worker function). These names are
// emitted by the engine via `tracing::info_span!` with `otel.name = ...`
// (see `motia/engine/src/engine/mod.rs:480` and
// `motia/engine/src/invocation/mod.rs:96`).
//
// We do NOT gate on `service_name === 'iii'`: the engine defaults to
// `service_name = 'iii'` but operators can (and do) override it via
// `OTEL_SERVICE_NAME` or `otel.service_name` in `config.yaml`, in which
// case the original `service_name === 'iii'` check silently fails and
// the "hide engine routing" toggle becomes a no-op. The verb prefixes
// are distinctive enough on their own — user code does not name spans
// `handle_invocation X` or `call X`.
const ENGINE_VERB_PREFIXES = ['handle_invocation ', 'call '] as const

export interface SpanKindIndicator {
  icon: string
  label: string
}

export function getSpanKindIndicator(
  kind: string | undefined,
): SpanKindIndicator | null {
  if (!kind) return null
  const k = kind.toLowerCase()
  switch (k) {
    case 'server':
      return { icon: '▶', label: 'server (handles incoming)' }
    case 'client':
      return { icon: '↗', label: 'client (outgoing call)' }
    case 'producer':
      return { icon: '↥', label: 'producer (sends to queue)' }
    case 'consumer':
      return { icon: '↧', label: 'consumer (reads from queue)' }
    case 'internal':
      return { icon: '•', label: 'internal' }
    default:
      return null
  }
}

export function formatSpanLabel(
  span: Pick<VisualizationSpan, 'name' | 'service_name'>,
): string {
  let label = span.name
  for (const prefix of ENGINE_VERB_PREFIXES) {
    if (label.startsWith(prefix)) {
      label = label.slice(prefix.length)
      break
    }
  }
  if (span.service_name) {
    const workerPrefix = `${span.service_name}.`
    if (label.startsWith(workerPrefix)) {
      label = label.slice(workerPrefix.length)
    }
  }
  return label
}

// We keep `service_name` in the `Pick<...>` so existing callers and
// fixtures continue to compile — the predicate just no longer uses it.
//
// The name prefix alone is NOT sufficient: the harness SDK emits its own
// client span literally named `call <fn>` (worker `harness`) alongside the
// engine's `call <fn>` invocation span. Gating only on the prefix swept up
// those worker spans too, collapsing legitimate harness work. Engine-emitted
// routing spans carry a `function_id` attribute (set by the engine's
// invocation instrumentation, see `engine/src/invocation/mod.rs`); worker
// `call <fn>` spans do not — so we additionally require that marker.
export function isEngineRoutingSpan(
  span: Pick<VisualizationSpan, 'name' | 'service_name'> & {
    attributes?: Record<string, unknown>
  },
): boolean {
  if (!ENGINE_VERB_PREFIXES.some((p) => span.name.startsWith(p))) return false
  return span.attributes?.function_id != null
}

export function isEngineRoutingPair(
  parent: Pick<VisualizationSpan, 'name' | 'service_name'>,
  child: Pick<VisualizationSpan, 'name' | 'service_name'>,
): boolean {
  if (!parent.name.startsWith('handle_invocation ')) return false
  if (!child.name.startsWith('call ')) return false
  return (
    parent.name.slice('handle_invocation '.length) ===
    child.name.slice('call '.length)
  )
}
