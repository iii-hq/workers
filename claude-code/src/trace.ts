/**
 * Trace identity for a Claude Code turn — the same keys the agent harness
 * stamps, so the console's trace views group this worker's turns exactly like
 * a native harness turn.
 *
 * The mechanism is W3C baggage, not a custom protocol. `iii-sdk` initialises
 * OTel with a span processor that copies every baggage entry onto each span
 * STARTED inside the scope, and the engine propagates baggage across calls —
 * so one scope around a turn labels the turn's own span, the calls it makes,
 * and the spans those produce in other workers.
 *
 * The keys (workers/console/docs/timeline-span-tags.md, harness
 * `src/functions/turn.rs`):
 *
 * | Key | What it carries |
 * |---|---|
 * | `iii.session.id` | the session — what the traces view groups by |
 * | `iii.message.id` | one turn; one grouped "message" in that view |
 * | `iii.session.name` | the session's display title |
 * | `iii.tag.kind` | classifies the span: `claude.run` / `claude.terminal.turn` |
 * | `iii.tag.message` | a preview of what was asked |
 * | `iii.tag.display_name` | the label a timeline shows instead of the span name |
 *
 * An explicit inner span is required, not optional: baggage lands only on
 * spans that START inside the scope, so a scope with no span of its own
 * labels nothing and the turn has no tag root.
 */

import { type Context, SpanStatusCode, context, propagation, trace } from '@opentelemetry/api';

/** How a turn reached this worker — the `iii.tag.kind` value it gets. */
export type TurnKind = 'claude.run' | 'claude.terminal.turn';

export type TurnIdentity = {
  /** The iii session id. Group-by key in the traces view. */
  sessionId: string;
  /** One turn. Absent for a scope that is not a single turn. */
  turnId?: string;
  kind: TurnKind;
  /** Session title, when the worker knows one. */
  sessionName?: string;
  /** What was asked, for the trace label. Trimmed to a preview here. */
  message?: string;
  /** Overrides the span label. Omit when the span name already says it. */
  displayName?: string;
};

const PREVIEW_CHARS = 120;

/** One line, bounded — a trace label, never a transcript. */
export function preview(text: string, limit = PREVIEW_CHARS): string {
  const line = text.replace(/\s+/g, ' ').trim();
  return line.length > limit ? `${line.slice(0, limit - 1)}…` : line;
}

/**
 * The identity keys for one turn. Exported because this mapping — which key
 * carries which value — IS the contract with the trace views; the plumbing
 * around it is OTel's.
 */
export function identityBaggage(identity: TurnIdentity): Record<string, string> {
  const entries: Record<string, string> = {
    'iii.session.id': identity.sessionId,
    'iii.tag.kind': identity.kind,
  };
  if (identity.turnId) entries['iii.message.id'] = identity.turnId;
  if (identity.sessionName) entries['iii.session.name'] = identity.sessionName;
  const message = identity.message ? preview(identity.message) : '';
  if (message) entries['iii.tag.message'] = message;
  if (identity.displayName) entries['iii.tag.display_name'] = preview(identity.displayName, 64);
  return entries;
}

function withBaggage(entries: Record<string, string>): Context {
  let baggage = propagation.getActiveBaggage() ?? propagation.createBaggage();
  for (const [key, value] of Object.entries(entries)) {
    baggage = baggage.setEntry(key, { value });
  }
  return propagation.setBaggage(context.active(), baggage);
}

/**
 * Run `work` as one traced turn: the identity in baggage, and a span that
 * starts inside it so the turn has a span of its own to carry the tags.
 *
 * Tracing never changes an answer. A tracing failure is swallowed and the
 * work runs untraced rather than failing a turn over a label.
 */
export async function runTurnSpan<T>(
  name: string,
  identity: TurnIdentity,
  work: () => Promise<T>,
): Promise<T> {
  let scope: Context;
  try {
    scope = withBaggage(identityBaggage(identity));
  } catch (err) {
    console.warn(`trace scope failed (${name}): ${String(err)}`);
    return work();
  }
  return context.with(scope, async () => {
    const span = trace.getTracer('claude-code').startSpan(name);
    return context.with(trace.setSpan(context.active(), span), async () => {
      try {
        const result = await work();
        span.end();
        return result;
      } catch (err) {
        span.setStatus({ code: SpanStatusCode.ERROR, message: String(err) });
        span.setAttribute('iii.tag.outcome', 'failed');
        span.end();
        throw err;
      }
    });
  });
}
