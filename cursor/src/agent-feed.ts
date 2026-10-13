/**
 * Owned event feeds: `cursor::agent-event` (normalized AgentEvent frames) and
 * `cursor::raw-event` (raw Cursor ACP / Bridge frames).
 *
 * Each feed is a trigger type this worker registers and serves. A consumer
 * binds it with `{ session_id, metadata? }` and receives one Void call per
 * frame of that session: `{ session_id, event_id, seq, epoch, source, event }`.
 *
 * - `seq` is contiguous per (feed, session_id, epoch) from 0; each feed keeps
 *   its own counter.
 * - `epoch` is `<process uuid>-<generation uuid>`. `releaseEmitterSequence`
 *   drops a session's counters at the end of a turn, so the next frame starts a
 *   new generation (a new epoch string) at seq 0.
 * - `event_id` is the stable `cursor-...` id when the caller has one (replayed
 *   Bridge/ACP frames keep the same id across retries and restarts, so a
 *   consumer deduplicates them); otherwise `<session_id>-<epoch>-<seq:08>`.
 *
 * Frames are ephemeral: not stored, not replayed. The durable record of a turn
 * is the session record behind `cursor::status` and the result `cursor::run`
 * returns.
 */

import { randomUUID } from 'node:crypto';
import { type IIIClient, TriggerAction } from 'iii-sdk';

export const MAX_BINDINGS = 256;
export const MAX_SESSION_ID_LENGTH = 512;
export const AGENT_EVENT_TRIGGER_TYPE = 'cursor::agent-event';
export const RAW_EVENT_TRIGGER_TYPE = 'cursor::raw-event';
export const FEED_SOURCE = 'cursor';

const CONFIG_KEYS = new Set(['session_id', 'metadata']);
const PROCESS_EPOCH = randomUUID();

type SequenceState = { generation: string; sequence: number };

/** Every feed's per-session counters, so one release resets all feeds. */
const sequenceStores = new Set<Map<string, SequenceState>>();

/**
 * Drop a session's sequence on every feed. The next frame for the session
 * starts a new generation, i.e. a new `epoch` string, at seq 0.
 */
export function releaseEmitterSequence(sessionId: string): void {
  for (const store of sequenceStores) store.delete(sessionId);
}

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

/**
 * Emit one frame. Resolves `true` when at least one bound consumer accepted
 * the delivery, `false` when nobody is bound for the session or every
 * delivery failed. Callers that track what a consumer has seen (streamed text
 * deltas, `body_streamed`) only advance on `true`, so a frame nobody received
 * is folded into the next delta or the final message body instead of being
 * lost. Never throws.
 */
export type FeedEmit = (
  sessionId: string,
  event: unknown,
  stableEventId?: string,
) => Promise<boolean>;

/** What CursorWorker / RunAccumulator accept; anything but `false` counts as delivered. */
export type Emit = (sessionId: string, event: unknown, stableEventId?: string) => Promise<unknown>;

export type Feed = { emit: FeedEmit; bindingCount: () => number };

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
  const sequenceBySession = new Map<string, SequenceState>();
  sequenceStores.add(sequenceBySession);

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

  async function emit(sessionId: string, event: unknown, stableEventId?: string): Promise<boolean> {
    const state = sequenceBySession.get(sessionId) ?? { generation: randomUUID(), sequence: 0 };
    sequenceBySession.set(sessionId, { ...state, sequence: state.sequence + 1 });
    const epoch = `${PROCESS_EPOCH}-${state.generation}`;
    const seq = state.sequence;
    const payload: FeedPayload = {
      session_id: sessionId,
      event_id: stableEventId ?? `${sessionId}-${epoch}-${String(seq).padStart(8, '0')}`,
      seq,
      epoch,
      source: opts.source,
      event,
    };
    let delivered = 0;
    for (const binding of Array.from(bindings.values())) {
      if (binding.config.session_id !== sessionId) continue;
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
      } catch (error) {
        console.warn(
          `${opts.id}: delivery to ${binding.function_id} failed for ${sessionId}: ${safeError(error)}`,
        );
      }
    }
    return delivered > 0;
  }

  return { emit, bindingCount: () => bindings.size };
}

/** The worker's two feeds, each with its own per-session sequence. */
export function createAgentFeeds(iii: IIIClient): { emit: FeedEmit; emitRaw: FeedEmit } {
  const agent = createFeed(iii, {
    id: AGENT_EVENT_TRIGGER_TYPE,
    source: FEED_SOURCE,
    description:
      'Normalized AgentEvent frames of one Cursor session. Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  const raw = createFeed(iii, {
    id: RAW_EVENT_TRIGGER_TYPE,
    source: FEED_SOURCE,
    description:
      'Raw Cursor ACP / SDK Bridge frames of one session, unchanged. Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  return { emit: agent.emit, emitRaw: raw.emit };
}

function safeError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.replaceAll(/(?:key|token|secret)_[A-Za-z0-9._-]+/gi, '<redacted>');
}
