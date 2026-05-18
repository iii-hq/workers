/**
 * Merge-rule implementations. Ports
 * `hook-fanout/src/lib.rs::merge_*` verbatim. Pure JSON manipulation —
 * easy to unit-test without a live engine.
 */

export function mergeFirstBlockWins(replies: unknown[]): Record<string, unknown> {
  for (const reply of replies) {
    if (
      typeof reply === 'object' &&
      reply !== null &&
      (reply as Record<string, unknown>).block === true
    ) {
      const r = reply as Record<string, unknown>;
      const out: Record<string, unknown> = {
        block: true,
        reason: r.reason ?? null,
      };
      // Preserve denial envelope when subscriber returned one — used by
      // approval-gate to thread DenialEnvelope through to the orchestrator.
      if (r.denial !== undefined) out.denial = r.denial;
      return out;
    }
  }
  return { block: false };
}

export function mergeFieldMerge(initial: unknown, replies: unknown[]): unknown {
  const out: Record<string, unknown> =
    typeof initial === 'object' && initial !== null
      ? { ...(initial as Record<string, unknown>) }
      : {};
  for (const reply of replies) {
    if (typeof reply !== 'object' || reply === null) continue;
    const r = reply as Record<string, unknown>;
    if (r.content !== undefined) out.content = r.content;
    if (r.details !== undefined) {
      const existing = out.details;
      if (
        typeof existing === 'object' &&
        existing !== null &&
        typeof r.details === 'object' &&
        r.details !== null
      ) {
        out.details = {
          ...(existing as Record<string, unknown>),
          ...(r.details as Record<string, unknown>),
        };
      } else if (typeof r.details === 'object' && r.details !== null) {
        out.details = r.details;
      }
    }
    if (typeof r.terminate === 'boolean') out.terminate = r.terminate;
  }
  return out;
}

export function mergePipelineLastWins(original: unknown, replies: unknown[]): unknown {
  let last: unknown | undefined;
  for (const reply of replies) {
    const transformed = decodeTransformMessages(reply);
    if (transformed !== undefined) last = transformed;
  }
  return last !== undefined ? last : original;
}

function decodeTransformMessages(reply: unknown): unknown | undefined {
  if (Array.isArray(reply)) return reply;
  if (typeof reply === 'object' && reply !== null) {
    const messages = (reply as Record<string, unknown>).messages;
    if (Array.isArray(messages)) return messages;
  }
  return undefined;
}
