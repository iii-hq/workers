/**
 * `redact`, applied to one value — the identity function when `redact` is
 * absent (nothing claims the span's function id; see `spanRawRedactor` in
 * `functionTriggerFromSpan.ts`). The primitive both helpers below, and any
 * tab reading a single ad-hoc field (`SpanErrorsTab`'s message/type/stack)
 * rather than a whole attribute bag, build on — one seam, so render and copy
 * can never disagree about what is safe to show.
 */
export function redactValue(
  value: unknown,
  redact?: (value: unknown) => unknown,
): unknown {
  return redact ? redact(value) : value
}

/**
 * `Object.entries(attributes)` with `redact` applied to each VALUE — the
 * single seam `SpanTagsTab` and `SpanLogsTab` route both their rendered text
 * and (for tags) their click-to-copy value through, so the two exits cannot
 * disagree about what is safe to show.
 *
 * Both tabs render the same span data `functionTriggerFromSpan.ts` reads to
 * build the info tab's `FunctionTriggerCard` (`iii.payload.json` event
 * attributes, `tool.arguments`) — so whenever that card's raw pane has a
 * redactor, these siblings need the identical one applied to their own
 * copies of the same data. `redact` is `undefined` when nothing claims the
 * span's function id, in which case every value passes through unchanged.
 */
export function redactAttributeEntries(
  attributes: Record<string, unknown> | undefined,
  redact?: (value: unknown) => unknown,
): [string, unknown][] {
  return Object.entries(attributes ?? {}).map(([key, value]) => [
    key,
    redactValue(value, redact),
  ])
}
