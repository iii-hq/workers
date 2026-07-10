/**
 * Page-level grouping for the trace timeline's filter menu. The generic
 * mechanics (ranked groups + subtree hiding + the floating funnel menu)
 * live with the component in `components/timeline`; THIS module decides
 * what a "span group" means for the traces page.
 *
 * Groups are the OWNING function id, resolved in this order:
 *
 * 1. the span's own explicit identity (`faas.invoked_name`/`function_id`
 *    attrs, or a worker-SDK `execute <fn>` span name) — that span IS the
 *    function's machinery, so one menu entry covers a whole call family:
 *    hiding `session::update-message` removes every call of it, and
 *    high-frequency bookkeeping like that naturally ranks at the top;
 * 2. tag ROOTS (`iii.tag.kind` starting a scope, e.g. `harness::turn step`
 *    — see workers/console/docs/timeline-span-tags.md) group under their
 *    own span NAME. They are producer-declared first-class segments, not
 *    machinery, so they get their own menu entry instead of vanishing with
 *    the function whose baggage they inherit — hiding `harness::turn`
 *    hides the queue/execute wrappers but keeps the step span (which for
 *    sub-agents carries the task title);
 * 3. the baggage-stamped `iii.function.id` as attribution fallback;
 * 4. the operation name, so everything stays hideable.
 *
 * Tag-root detection needs the parent's attributes, so callers that have
 * the whole trace should pass a `spansById` map (the detail views do —
 * they bind it once per trace). Without it, rule 2 is skipped and grouping
 * degrades to the historical explicit/baggage/name behavior.
 */

import { explicitFunctionId } from './functionCallFromSpan'
import { inheritedTags, tagRootKind } from './spanLabel'
import type { VisualizationSpan } from './traceTransform'

export function traceSpanGroupKey(
  span: VisualizationSpan,
  spansById?: ReadonlyMap<string, VisualizationSpan>,
): string {
  const explicit = explicitFunctionId(span)
  if (explicit) return explicit

  if (spansById) {
    const inherited = inheritedTags(span.parent_span_id, (id) =>
      spansById.get(id),
    )
    if (tagRootKind(span.attributes, inherited.kind)) return span.name
  }

  const baggage = span.attributes?.['iii.function.id']
  if (typeof baggage === 'string' && baggage.length > 0) return baggage
  return span.name
}
