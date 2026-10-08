/**
 * Owned event feeds: `opencode::agent-event` (translated AgentEvent frames)
 * and `opencode::raw-event` (verbatim OpenCode JSON events).
 *
 * Each feed is a trigger type this worker registers and serves. A consumer
 * binds it with `{ session_id, metadata? }` and receives one Void call per
 * frame of that session:
 * `{ session_id, event_id, seq, epoch, source, event }`. `seq` is contiguous
 * per (feed, session_id, epoch) from 0; `epoch` is a per-process uuid, so a
 * restart starts a new sequence. Frames are ephemeral: nothing is stored or
 * replayed, a consumer bound late never sees earlier frames. The durable
 * record of a turn is `opencode::status` / `opencode::sessions::list` and the
 * result `opencode::run` returns.
 */

import { randomUUID } from 'node:crypto';
import { type IIIClient, TriggerAction } from 'iii-sdk';

export const MAX_BINDINGS = 256;
export const MAX_SESSION_ID_LENGTH = 512;
export const AGENT_EVENT_TRIGGER_TYPE = 'opencode::agent-event';
export const RAW_EVENT_TRIGGER_TYPE = 'opencode::raw-event';
export const FEED_SOURCE = 'opencode';

const CONFIG_KEYS = new Set(['session_id', 'metadata']);

export type FeedConfig = { session_id: string; metadata?: Record<string, unknown> };

export type FeedPayload = {
  session_id: string;
  event_id: string;
  seq: number;
  epoch: string;
  source: string;
  event: unknown;
};

type Binding = {
  id: string;
  function_id: string;
  namespace?: string;
  metadata?: unknown;
  config: FeedConfig;
};

/** Emit one frame; resolves to the number of consumers the frame was handed to. */
export type Emit = (session_id: string, event: unknown) => Promise<number>;

export type Feed = { emit: Emit; bindingCount: () => number };

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Validate a binding config; throwing rejects the binding. */
export function validateFeedConfig(typeId: string, raw: unknown): FeedConfig {
  if (!isPlainObject(raw)) {
    throw new Error(`${typeId}: config must be an object with a session_id`);
  }
  for (const key of Object.keys(raw)) {
    if (!CONFIG_KEYS.has(key)) throw new Error(`${typeId}: unknown config key "${key}"`);
  }
  const sessionId = raw.session_id;
  if (typeof sessionId !== 'string' || sessionId.length === 0) {
    throw new Error(`${typeId}: session_id must be a non-empty string`);
  }
  if (sessionId.length > MAX_SESSION_ID_LENGTH) {
    throw new Error(`${typeId}: session_id exceeds ${MAX_SESSION_ID_LENGTH} characters`);
  }
  if (raw.metadata === undefined) return { session_id: sessionId };
  if (!isPlainObject(raw.metadata)) throw new Error(`${typeId}: metadata must be an object`);
  return { session_id: sessionId, metadata: raw.metadata };
}

/** Register one owned feed trigger type and return its emitter. */
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
      async registerTrigger(binding) {
        const config = validateFeedConfig(opts.id, binding.config);
        if (!bindings.has(binding.id) && bindings.size >= MAX_BINDINGS) {
          throw new Error(`${opts.id}: binding limit (${MAX_BINDINGS}) reached`);
        }
        bindings.set(binding.id, {
          id: binding.id,
          function_id: binding.function_id,
          namespace: binding.namespace,
          metadata: binding.metadata,
          config,
        });
      },
      async unregisterTrigger(binding) {
        bindings.delete(binding.id);
      },
    },
  );

  async function emit(session_id: string, event: unknown): Promise<number> {
    const seq = seqBySession.get(session_id) ?? 0;
    seqBySession.set(session_id, seq + 1);
    const payload: FeedPayload = {
      session_id,
      event_id: `${session_id}-${epoch}-${String(seq).padStart(8, '0')}`,
      seq,
      epoch,
      source: opts.source,
      event,
    };
    let delivered = 0;
    for (const binding of Array.from(bindings.values())) {
      if (binding.config.session_id !== session_id) continue;
      const metadata = binding.config.metadata ?? binding.metadata;
      try {
        await iii.trigger({
          function_id: binding.function_id,
          namespace: binding.namespace,
          payload,
          ...(metadata === undefined ? {} : { metadata }),
          action: TriggerAction.Void(),
        });
        delivered++;
      } catch (err) {
        console.warn(`${opts.id}: delivery to ${binding.function_id} failed: ${String(err)}`);
      }
    }
    return delivered;
  }

  return { emit, bindingCount: () => bindings.size };
}

/** The worker's two feeds, each with its own per-session sequence. */
export function createAgentFeeds(iii: IIIClient): { emit: Emit; emitRaw: Emit } {
  const agent = createFeed(iii, {
    id: AGENT_EVENT_TRIGGER_TYPE,
    source: FEED_SOURCE,
    description:
      'Translated AgentEvent frames of one OpenCode session. Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  const raw = createFeed(iii, {
    id: RAW_EVENT_TRIGGER_TYPE,
    source: FEED_SOURCE,
    description:
      'Raw OpenCode JSON events (verbatim `opencode run --format json` lines) of one session. Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  return { emit: agent.emit, emitRaw: raw.emit };
}
