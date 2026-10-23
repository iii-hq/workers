/**
 * The worker's two event feeds, as trigger types it owns (no iii-stream):
 *
 * - `pi::agent-event`: translated AgentEvent frames (what the console and
 *   the acp worker render).
 * - `pi::raw-event`: every Pi AgentSession event, verbatim.
 *
 * A consumer binds a function with `{ session_id, metadata? }` and receives
 * `{ session_id, event_id, seq, epoch, source, event }` for that session only.
 * Delivery is fire-and-forget (`TriggerAction.Void`), sequential, and never
 * fails the turn. Frames are ephemeral: nothing is stored or replayed, so a
 * consumer bound late never sees earlier frames; the durable history is the
 * session record (`pi::status`) and the session-manager transcript.
 *
 * `seq` is contiguous per (feed, session_id, epoch) from 0, with a separate
 * counter per feed; `epoch` is a per-process uuid, so consumers order by
 * `(epoch, seq)` and dedup by `event_id`.
 */

import { randomUUID } from 'node:crypto';
import { type IIIClient, TriggerAction } from 'iii-sdk';

export const SOURCE = 'pi';
export const AGENT_EVENT_TRIGGER = 'pi::agent-event';
export const RAW_EVENT_TRIGGER = 'pi::raw-event';

/** Bindings per trigger type; a new binding beyond it is rejected, not queued. */
export const MAX_BINDINGS = 256;
const MAX_SESSION_ID_LEN = 512;
const CONFIG_KEYS = new Set(['session_id', 'metadata']);

export type FeedConfig = { session_id: string; metadata?: Record<string, unknown> };

type Binding = {
  id: string;
  function_id: string;
  namespace?: string;
  metadata?: Record<string, unknown>;
  config: FeedConfig;
};

export type FeedPayload = {
  session_id: string;
  event_id: string;
  seq: number;
  epoch: string;
  source: string;
  event: unknown;
};

/** Publish one frame for a session; resolves to the number of deliveries. */
export type Emit = (session_id: string, event: unknown) => Promise<unknown>;

export type Feed = {
  emit: (session_id: string, event: unknown, stableEventId?: string) => Promise<number>;
  bindingCount: () => number;
};

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Validate a binding config; throwing rejects the binding. */
export function validateFeedConfig(typeId: string, raw: unknown): FeedConfig {
  if (!isPlainObject(raw)) {
    throw new Error(`${typeId}: config must be an object with a session_id`);
  }
  for (const key of Object.keys(raw)) {
    if (!CONFIG_KEYS.has(key)) {
      throw new Error(`${typeId}: unknown config key \`${key}\` (allowed: session_id, metadata)`);
    }
  }
  const { session_id, metadata } = raw;
  if (typeof session_id !== 'string' || session_id.length === 0) {
    throw new Error(`${typeId}: session_id must be a non-empty string`);
  }
  if (session_id.length > MAX_SESSION_ID_LEN) {
    throw new Error(`${typeId}: session_id is longer than ${MAX_SESSION_ID_LEN} characters`);
  }
  if (metadata !== undefined && !isPlainObject(metadata)) {
    throw new Error(`${typeId}: metadata must be an object`);
  }
  return metadata === undefined ? { session_id } : { session_id, metadata };
}

/** Register one owned trigger type and return its emitter. */
export function createFeed(
  iii: IIIClient,
  opts: { id: string; source: string; description: string },
): Feed {
  const bindings = new Map<string, Binding>();
  const seqBySession = new Map<string, number>();
  const epoch = randomUUID();

  iii.registerTriggerType<unknown>(
    { id: opts.id, description: opts.description },
    {
      async registerTrigger(b) {
        const config = validateFeedConfig(opts.id, b.config);
        if (!bindings.has(b.id) && bindings.size >= MAX_BINDINGS) {
          throw new Error(`${opts.id}: binding limit (${MAX_BINDINGS}) reached`);
        }
        bindings.set(b.id, {
          id: b.id,
          function_id: b.function_id,
          namespace: b.namespace,
          metadata: b.metadata,
          config,
        });
      },
      async unregisterTrigger(b) {
        bindings.delete(b.id);
      },
    },
  );

  async function emit(session_id: string, event: unknown, stableEventId?: string) {
    const seq = seqBySession.get(session_id) ?? 0;
    seqBySession.set(session_id, seq + 1);
    const payload: FeedPayload = {
      session_id,
      event_id: stableEventId ?? `${session_id}-${epoch}-${String(seq).padStart(8, '0')}`,
      seq,
      epoch,
      source: opts.source,
      event,
    };
    let delivered = 0;
    for (const b of Array.from(bindings.values())) {
      if (b.config.session_id !== session_id) continue;
      const metadata = b.config.metadata ?? b.metadata;
      try {
        await iii.trigger({
          function_id: b.function_id,
          ...(b.namespace === undefined ? {} : { namespace: b.namespace }),
          payload,
          ...(metadata === undefined ? {} : { metadata }),
          action: TriggerAction.Void(),
        });
        delivered++;
      } catch (err) {
        console.warn(`${opts.id}: delivery to ${b.function_id} failed: ${String(err)}`);
      }
    }
    return delivered;
  }

  return { emit, bindingCount: () => bindings.size };
}

/** Both feeds, each with its own binding table and sequence counter. */
export function registerAgentFeeds(iii: IIIClient): { agent: Feed; raw: Feed } {
  const agent = createFeed(iii, {
    id: AGENT_EVENT_TRIGGER,
    source: SOURCE,
    description:
      'Translated AgentEvent frames of one Pi session (pi::run, pi::start, pi::task, run::start_and_wait and the console terminal). Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  const raw = createFeed(iii, {
    id: RAW_EVENT_TRIGGER,
    source: SOURCE,
    description:
      'Raw Pi AgentSession events (agent_start/end, turn_start/end, message_start/update/end, tool_execution_*, queue_update, compaction_*) of one session, verbatim. Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  return { agent, raw };
}
