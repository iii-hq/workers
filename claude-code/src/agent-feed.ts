/**
 * The worker's two event feeds, as trigger types it owns (no iii-stream):
 *
 * - `claude::agent-event`: translated AgentEvent frames (what the console and
 *   the acp worker render).
 * - `claude::raw-event`: every Claude Code message, verbatim.
 *
 * A consumer binds a function with `{ session_id, metadata? }` and receives
 * `{ session_id, event_id, seq, epoch, source, event }` for that session only.
 * Delivery is fire-and-forget (`TriggerAction.Void`), sequential, and never
 * fails the turn. Frames are ephemeral: nothing is stored or replayed, so a
 * consumer bound late never sees earlier frames; the durable history is the
 * session record (`claude::status`) and the session-manager transcript.
 *
 * On an iii 0.25 engine each feed is also an owner-delivered observable
 * function (`claude::on-agent-event`, `claude::on-raw-event`): an observer
 * with args `{ session_id }` gets the same payload through `obs.call`. The
 * trigger types stay for 0.24 engines and old consumers; invoking an `on-*`
 * function directly is refused with FORBIDDEN.
 *
 * `seq` is contiguous per (feed, session_id, epoch) from 0, with a separate
 * counter per feed; `epoch` is a per-process uuid, so consumers order by
 * `(epoch, seq)` and dedup by `event_id`.
 */

import { randomUUID } from 'node:crypto';
import {
  type IIIClient,
  InvocationError,
  ObserveRejection,
  type Observer,
  TriggerAction,
} from 'iii-sdk';

export const SOURCE = 'claude';
export const AGENT_EVENT_TRIGGER = 'claude::agent-event';
export const RAW_EVENT_TRIGGER = 'claude::raw-event';
/** The observable functions (iii 0.25) next to the two trigger types. */
export const AGENT_EVENT_OBSERVABLE = 'claude::on-agent-event';
export const RAW_EVENT_OBSERVABLE = 'claude::on-raw-event';

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
  /** Set for an observer of the observable function (iii 0.25), absent for a trigger binding. */
  observer?: Observer;
};

/** Observers share the bindings map under a key no trigger binding id uses. */
const observerKey = (observerId: string) => `observer:${observerId}`;

/** The observers' args (`FeedConfig`), validated by the engine (`INVALID_ARGS`). */
const FEED_ARGS_FORMAT = {
  type: 'object' as const,
  properties: {
    session_id: {
      type: 'string',
      minLength: 1,
      maxLength: MAX_SESSION_ID_LEN,
      description: 'The session whose frames this observer receives.',
    },
    metadata: {
      type: 'object',
      description:
        'Accepted for parity with the trigger config; an observer gets its sidecar from its own `metadata` option.',
    },
  },
  required: ['session_id'],
  additionalProperties: false,
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

/** Register one owned trigger type and its observable function; return the emitter. */
export function createFeed(
  iii: IIIClient,
  opts: { id: string; observableId: string; source: string; description: string },
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

  const forbidden = `${opts.observableId} is an event source: observe it (iii.observe({ function_id: '${opts.observableId}', args: { session_id } }, target)); only the claude-code worker emits its events`;

  // The iii 0.25 observable function next to the trigger type: owner-delivered,
  // its observers filed in the same bindings map (keyed apart from binding ids),
  // so one emit reaches bindings and observers alike. Events are routed
  // in-process, never through the function: invoking it is refused.
  iii.registerFunction(
    opts.observableId,
    async () => {
      throw new InvocationError({
        code: 'FORBIDDEN',
        message: forbidden,
        function_id: opts.observableId,
      });
    },
    {
      description: `${opts.description} Observe with iii.observe({ function_id: '${opts.observableId}', args: { session_id } }, target) (iii 0.25); invoking it is refused with FORBIDDEN.`,
      observe: {
        delivery: 'owner',
        args_format: FEED_ARGS_FORMAT,
        async onObserverAdded(obs) {
          let config: FeedConfig;
          try {
            config = validateFeedConfig(opts.observableId, obs.args);
          } catch (err) {
            throw new ObserveRejection(
              'INVALID_ARGS',
              err instanceof Error ? err.message : String(err),
            );
          }
          const key = observerKey(obs.id);
          if (!bindings.has(key) && bindings.size >= MAX_BINDINGS) {
            throw new ObserveRejection(
              'BINDING_LIMIT',
              `${opts.observableId}: binding limit (${MAX_BINDINGS}) reached`,
            );
          }
          bindings.set(key, { id: key, function_id: obs.target, config, observer: obs });
        },
        async onObserverRemoved(obs) {
          bindings.delete(observerKey(obs.id));
        },
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
      if (b.observer) {
        try {
          await b.observer.call(payload);
          delivered++;
        } catch (err) {
          const code = (err as { code?: unknown } | null)?.code;
          if (code === 'FILTERS_NOT_MATCHED' || code === 'OBSERVER_NOT_ACTIVE') continue;
          console.warn(`${opts.observableId}: delivery to ${b.function_id} failed: ${String(err)}`);
        }
        continue;
      }
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

/** Both feeds, each with its own binding table (bindings and observers) and sequence counter. */
export function registerAgentFeeds(iii: IIIClient): { agent: Feed; raw: Feed } {
  const agent = createFeed(iii, {
    id: AGENT_EVENT_TRIGGER,
    observableId: AGENT_EVENT_OBSERVABLE,
    source: SOURCE,
    description:
      'Translated AgentEvent frames of one Claude Code session (claude::run, claude::start, claude::task, run::start_and_wait and the console terminal). Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  const raw = createFeed(iii, {
    id: RAW_EVENT_TRIGGER,
    observableId: RAW_EVENT_OBSERVABLE,
    source: SOURCE,
    description:
      'Raw Claude Code messages (system/init, assistant, user, result, stream_event) of one session, verbatim. Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  return { agent, raw };
}
