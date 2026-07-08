/**
 * Page-level grouping for the trace timeline's filter menu. The generic
 * mechanics (ranked groups + subtree hiding + the floating funnel menu)
 * live with the component in `components/timeline`; THIS module decides
 * what a "span group" means for the traces page.
 *
 * Groups are the OWNING function id (`spanFunctionId`: explicit
 * `faas.invoked_name`/`function_id` attrs first, baggage-stamped
 * `iii.function.id` as fallback), so one menu entry covers a whole call
 * family — hiding `session::update-message` removes every call of it, and
 * high-frequency bookkeeping like that naturally ranks at the top of the
 * menu. Spans with no function attribution group under their operation
 * name so everything stays hideable.
 */

import { spanFunctionId } from './functionCallFromSpan'
import type { VisualizationSpan } from './traceTransform'

export function traceSpanGroupKey(span: VisualizationSpan): string {
  return spanFunctionId(span) ?? span.name
}
