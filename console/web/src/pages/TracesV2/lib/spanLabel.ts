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

// Verb prefixes stripped for DISPLAY only. Includes the worker SDK's
// `execute <fn>` handler-span prefix (see `functionTriggerFromSpan.ts`), which
// must NOT join ENGINE_VERB_PREFIXES: execute spans are worker work, and
// classifying them as engine routing would dim/hide them under the
// "hide engine routing" toggle.
const DISPLAY_STRIP_PREFIXES = [...ENGINE_VERB_PREFIXES, 'execute '] as const

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
  for (const prefix of DISPLAY_STRIP_PREFIXES) {
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

// The "relevant span" tagging convention (see
// workers/console/docs/timeline-span-tags.md): a producing worker stamps
// `iii.tag.kind`/`iii.tag.display_name` as OTel baggage, which the (>=
// 0.21.2-next.1) BaggageSpanProcessor copies onto every span attribute set
// in scope, same mechanism as the existing `iii.tag.message`/
// `iii.session.name` trace tags — plain W3C baggage, nothing custom-protocol.
const TAG_KIND_ATTR = 'iii.tag.kind'
const TAG_DISPLAY_NAME_ATTR = 'iii.tag.display_name'
const TAG_HIDDEN_ATTR = 'iii.tag.hidden'

/**
 * The INTERNAL family of a span, when a producer tagged the call site
 * `iii.tag.hidden = <family>` (baggage — e.g. the harness's `state::*`
 * bookkeeping, session-manager's event fan-out). Internal spans form their
 * own section of the span filter and are hidden by default; the value is
 * the section's entry label ("harness state", "session events"). The
 * baggage smear deliberately carries the tag to the whole delivery subtree
 * — every tagged span hides, no root/echo distinction.
 */
export function internalFamilyOf(
  attributes: Record<string, unknown> | undefined,
): string | null {
  const family = attributes?.[TAG_HIDDEN_ATTR]
  return typeof family === 'string' && family.length > 0 ? family : null
}

/** `iii.tag.kind` off a span's attributes, if a producer set one. */
export function tagKindOf(
  span: Pick<VisualizationSpan, 'name' | 'service_name'> & {
    attributes?: Record<string, unknown>
  },
): string | undefined {
  const value = span.attributes?.[TAG_KIND_ATTR]
  return typeof value === 'string' ? value : undefined
}

/** The `iii.tag.*` values a span inherits from its ancestry — each taken
 *  from the NEAREST ancestor that carries that attribute. */
export interface InheritedTags {
  kind?: string
  displayName?: string
}

/** The parent-chain shape `inheritedTags` walks. `VisualizationSpan` and the
 *  strip's shaped stored spans both satisfy it. */
export interface TagCarrier {
  attributes?: Record<string, unknown>
  parent_span_id?: string
}

/** Guard against malformed parent chains; real traces are far shallower. */
const MAX_TAG_WALK = 1000

/**
 * Resolve the `iii.tag.kind` / `iii.tag.display_name` a span INHERITS: walk
 * the parent chain and take each value from the nearest ancestor carrying
 * it. The immediate parent is NOT enough — a worker on an older SDK whose
 * span processor drops `iii.tag.*` baggage leaves gap spans in the middle
 * of a scope (e.g. `execute context::assemble` carries no tags while its
 * `router::models::get` child re-materializes them), and comparing against
 * such a gap would misread a smear echo as a fresh scope.
 */
export function inheritedTags(
  parentId: string | undefined,
  lookup: (id: string) => TagCarrier | undefined,
): InheritedTags {
  const out: InheritedTags = {}
  const seen = new Set<string>()
  let id = parentId
  for (let hops = 0; id && hops < MAX_TAG_WALK; hops++) {
    if (seen.has(id)) break
    seen.add(id)
    const node = lookup(id)
    if (!node) break
    const kind = node.attributes?.[TAG_KIND_ATTR]
    if (out.kind === undefined && typeof kind === 'string') out.kind = kind
    const display = node.attributes?.[TAG_DISPLAY_NAME_ATTR]
    if (out.displayName === undefined && typeof display === 'string') {
      out.displayName = display
    }
    if (out.kind !== undefined && out.displayName !== undefined) break
    id = node.parent_span_id
  }
  return out
}

/**
 * The `iii.tag.kind` value when this span is a tag ROOT — the span that
 * STARTS a tag scope, per the identity rule in
 * workers/console/docs/timeline-span-tags.md: it carries a kind its
 * ancestry does not already carry (`inheritedKind`, see [`inheritedTags`]).
 * Descendants repeating the kind (the baggage smear) are echoes and return
 * null. The producer's scope span (`harness::turn step`, a sub-agent step,
 * …) is the trace's first-class segment; grouping and hiding treat it as
 * its own thing rather than as machinery of the function whose baggage it
 * inherits.
 */
export function tagRootKind(
  attributes: Record<string, unknown> | undefined,
  inheritedKind: string | undefined,
): string | null {
  const kind = attributes?.[TAG_KIND_ATTR]
  if (typeof kind !== 'string' || kind.length === 0) return null
  return inheritedKind === kind ? null : kind
}

/**
 * Display label for a span, preferring a producer-supplied
 * `iii.tag.display_name` override before falling back to the usual
 * verb-stripped span name.
 *
 * The override only applies where it is NEW information — the display-name
 * equivalent of the tag-root rule in
 * workers/console/docs/timeline-span-tags.md: baggage copies `iii.tag.*`
 * onto every span started inside the scope, so a sub-agent turn's
 * `Sub-agent · <task>` name smears across all its LLM calls, session
 * writes, and tool spans. A span whose ancestry already carries the SAME
 * display name (`inheritedDisplayName`, see [`inheritedTags`]) is an echo
 * and keeps its own (verb-stripped) name; only the scope's first span —
 * where the name first appears — renders it. Without ancestry (no parent,
 * parent outside the window) the override applies, which is right for
 * roots and self-heals for echoes once the parent arrives.
 */
export function resolveSpanLabel(
  span: Pick<VisualizationSpan, 'name' | 'service_name'> & {
    attributes?: Record<string, unknown>
  },
  inheritedDisplayName?: string,
): string {
  const override = span.attributes?.[TAG_DISPLAY_NAME_ATTR]
  if (typeof override === 'string' && override !== inheritedDisplayName) {
    return override
  }
  return formatSpanLabel(span)
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
//
// Built-in calls are NOT routing: a `call <fn>` span with
// `iii.function.kind: "internal"` is an engine built-in executing
// in-process (`configuration::list`, `state::get`, …). There is no worker
// `execute` span behind it — the call span IS the invocation's only record
// (and carries its error status) — so hiding it as a "wrapper" would erase
// the call from the view entirely.
export function isEngineRoutingSpan(
  span: Pick<VisualizationSpan, 'name' | 'service_name'> & {
    attributes?: Record<string, unknown>
  },
): boolean {
  if (!ENGINE_VERB_PREFIXES.some((p) => span.name.startsWith(p))) return false
  if (span.attributes?.['iii.function.kind'] === 'internal') return false
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
