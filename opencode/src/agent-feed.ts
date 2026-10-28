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
 *
 * On an iii 0.25 engine each feed is also an owner-delivered observable
 * function (`opencode::on-agent-event`, `opencode::on-raw-event`): an observer
 * with args `{ session_id }` gets the same payload through `obs.call`. The
 * trigger types stay for 0.24 engines and old consumers; invoking an `on-*`
 * function directly is refused with FORBIDDEN.
 */

import { randomUUID } from 'node:crypto';
import {
  type IIIClient,
  InvocationError,
  ObserveRejection,
  type Observer,
  TriggerAction,
} from 'iii-sdk';

export const MAX_BINDINGS = 256;
export const MAX_SESSION_ID_LENGTH = 512;
export const AGENT_EVENT_TRIGGER_TYPE = 'opencode::agent-event';
export const RAW_EVENT_TRIGGER_TYPE = 'opencode::raw-event';
/** The observable functions (iii 0.25) next to the two trigger types. */
export const AGENT_EVENT_OBSERVABLE = 'opencode::on-agent-event';
export const RAW_EVENT_OBSERVABLE = 'opencode::on-raw-event';
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
      maxLength: MAX_SESSION_ID_LENGTH,
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

/**
 * Register one owned feed trigger type and, when `observableId` is given, its
 * observable function (iii 0.25); return the emitter.
 */
export function createFeed(
  iii: IIIClient,
  opts: { id: string; observableId?: string; source: string; description: string },
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

  if (opts.observableId !== undefined) {
    const observableId = opts.observableId;
    const forbidden = `${observableId} is an event source: observe it (iii.observe({ function_id: '${observableId}', args: { session_id } }, target)); only the opencode worker emits its events`;

    // The iii 0.25 observable function next to the trigger type: owner-delivered,
    // its observers filed in the same bindings map (keyed apart from binding ids),
    // so one emit reaches bindings and observers alike. Events are routed
    // in-process, never through the function: invoking it is refused.
    iii.registerFunction(
      observableId,
      async () => {
        throw new InvocationError({
          code: 'FORBIDDEN',
          message: forbidden,
          function_id: observableId,
        });
      },
      {
        description: `${opts.description} Observe with iii.observe({ function_id: '${observableId}', args: { session_id } }, target) (iii 0.25); invoking it is refused with FORBIDDEN.`,
        observe: {
          delivery: 'owner',
          args_format: FEED_ARGS_FORMAT,
          async onObserverAdded(obs) {
            let config: FeedConfig;
            try {
              config = validateFeedConfig(observableId, obs.args);
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
                `${observableId}: binding limit (${MAX_BINDINGS}) reached`,
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
  }

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
      if (binding.observer) {
        try {
          await binding.observer.call(payload);
          delivered++;
        } catch (err) {
          const code = (err as { code?: unknown } | null)?.code;
          if (code === 'FILTERS_NOT_MATCHED' || code === 'OBSERVER_NOT_ACTIVE') continue;
          console.warn(
            `${opts.observableId ?? opts.id}: delivery to ${binding.function_id} failed: ${String(err)}`,
          );
        }
        continue;
      }
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

/** The worker's two feeds, each with its own binding table (bindings and observers) and per-session sequence. */
export function createAgentFeeds(iii: IIIClient): { emit: Emit; emitRaw: Emit } {
  const agent = createFeed(iii, {
    id: AGENT_EVENT_TRIGGER_TYPE,
    observableId: AGENT_EVENT_OBSERVABLE,
    source: FEED_SOURCE,
    description:
      'Translated AgentEvent frames of one OpenCode session. Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  const raw = createFeed(iii, {
    id: RAW_EVENT_TRIGGER_TYPE,
    observableId: RAW_EVENT_OBSERVABLE,
    source: FEED_SOURCE,
    description:
      'Raw OpenCode JSON events (verbatim `opencode run --format json` lines) of one session. Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  return { emit: agent.emit, emitRaw: raw.emit };
}
