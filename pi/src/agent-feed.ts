/**
 * The worker's two event feeds, as private functions it hosts (no iii-stream):
 *
 * - `pi::on-agent-event`: translated AgentEvent frames (what the console and
 *   the acp worker render).
 * - `pi::on-raw-event`: every Pi AgentSession event, verbatim.
 *
 * A consumer observes a feed with args `{ session_id }` and receives
 * `{ session_id, event_id, seq, epoch, source, event }` for that session only:
 *
 *   iii.observe(
 *     { function_id: 'pi::on-agent-event', args: { session_id }, deliver: 'output' },
 *     'acp::on-event',
 *   )
 *
 * Each frame is a fire-and-forget call of the feed (`CallAction.Void`); iii
 * delivers it to the observers filed under its session id and never fails the
 * turn. A frame for a session nobody observes is not sent. Frames are
 * ephemeral: nothing is stored or replayed, so an observer added late never
 * sees earlier frames; the durable history is the session record
 * (`pi::status`) and the session-manager transcript.
 *
 * `seq` is contiguous per (feed, session_id, epoch) from 0, with a separate
 * counter per feed, and advances whether or not the frame is observed; `epoch`
 * is a per-process uuid, so consumers order by `(epoch, seq)` and dedup by
 * `event_id`.
 *
 * A frame carries a copy of the event taken when it is emitted. A feed is a
 * private function, and the SDK passes a private function's payload by
 * reference (deep-frozen under NODE_ENV=development), so the copy keeps the
 * caller's own objects, which it may still be mutating, out of the call.
 */

import { randomUUID } from 'node:crypto';
import { CallAction, type IIIClient, ObserveRejection } from 'iii-sdk';

export const SOURCE = 'pi';
export const ON_AGENT_EVENT_FN = 'pi::on-agent-event';
export const ON_RAW_EVENT_FN = 'pi::on-raw-event';

/** Observers per feed; one more is rejected with `OBSERVER_LIMIT`, not queued. */
export const MAX_OBSERVERS = 256;
export const MAX_SESSION_ID_LEN = 512;

/** What an observer of a feed passes: the session whose frames it receives. */
export type FeedArgs = { session_id: string };

export type FeedPayload = {
  session_id: string;
  event_id: string;
  seq: number;
  epoch: string;
  source: string;
  event: unknown;
};

/** `args_format` of both feeds; the engine rejects other args with `INVALID_ARGS`. */
export const FEED_ARGS_FORMAT = {
  type: 'object' as const,
  properties: {
    session_id: {
      type: 'string',
      minLength: 1,
      maxLength: MAX_SESSION_ID_LEN,
      description: 'The session whose frames this observer receives.',
    },
  },
  required: ['session_id'],
  additionalProperties: false,
};

/** `request_format` and `response_format` of both feeds: the frame. */
export const FEED_PAYLOAD_FORMAT = {
  type: 'object' as const,
  properties: {
    session_id: { type: 'string' },
    event_id: {
      type: 'string',
      description:
        'Unique per frame; `<session_id>-<epoch>-<seq:08>` unless the frame has a stable id.',
    },
    seq: {
      type: 'integer',
      minimum: 0,
      description: 'Contiguous per (feed, session_id, epoch) from 0.',
    },
    epoch: { type: 'string', description: 'Per-process uuid.' },
    source: { type: 'string', description: 'The agent: `pi`.' },
    event: { description: 'The AgentEvent frame, or the raw Pi event.' },
  },
  required: ['session_id', 'event_id', 'seq', 'epoch', 'source', 'event'],
};

/** Publish one frame for a session. */
export type Emit = (session_id: string, event: unknown) => Promise<unknown>;

export type Feed = {
  /** Resolves `true` when the frame was sent to the session's observers, `false` when nobody observes it. */
  emit: (session_id: string, event: unknown, stableEventId?: string) => Promise<boolean>;
};

/**
 * The event as it is at emit time, sharing no object with the caller:
 * `structuredClone`, or a JSON round trip (what the wire carries) for values
 * it cannot clone, such as functions.
 */
export function snapshot<T>(value: T): T {
  if (value === null || typeof value !== 'object') return value;
  try {
    return structuredClone(value);
  } catch {
    const text = JSON.stringify(value);
    return (text === undefined ? undefined : JSON.parse(text)) as T;
  }
}

/** Register one feed function and return its emitter. */
export function createFeed(
  iii: IIIClient,
  opts: { id: string; source: string; description: string },
): Feed {
  const seqBySession = new Map<string, number>();
  const epoch = randomUUID();

  const feed = iii
    .registerFunction<FeedPayload, FeedPayload>(opts.id, (frame) => frame, {
      description: opts.description,
      request_format: FEED_PAYLOAD_FORMAT,
      response_format: FEED_PAYLOAD_FORMAT,
      args_format: FEED_ARGS_FORMAT,
      private: true,
    })
    .onObserverAdded(() => {
      if (feed.observers().length >= MAX_OBSERVERS) {
        throw new ObserveRejection(
          'OBSERVER_LIMIT',
          `${opts.id}: observer limit (${MAX_OBSERVERS}) reached`,
        );
      }
    })
    .onObserverMatch({
      observer: (obs) => String((obs.args as FeedArgs).session_id),
      call: (call) => call.input.session_id,
    });

  async function emit(session_id: string, event: unknown, stableEventId?: string) {
    const seq = seqBySession.get(session_id) ?? 0;
    seqBySession.set(session_id, seq + 1);
    if (!iii.isObserved(opts.id, { input: { session_id } })) return false;
    const event_id = stableEventId ?? `${session_id}-${epoch}-${String(seq).padStart(8, '0')}`;
    try {
      const payload: FeedPayload = {
        session_id,
        event_id,
        seq,
        epoch,
        source: opts.source,
        event: snapshot(event),
      };
      await iii.call({ function_id: opts.id, payload, action: CallAction.Void() });
      return true;
    } catch (err) {
      console.warn(`${opts.id}: frame ${event_id} was not sent: ${String(err)}`);
      return false;
    }
  }

  return { emit };
}

/** Both feeds, each with its own observers and sequence counter. */
export function registerAgentFeeds(iii: IIIClient): { agent: Feed; raw: Feed } {
  const agent = createFeed(iii, {
    id: ON_AGENT_EVENT_FN,
    source: SOURCE,
    description:
      'Translated AgentEvent frames of one Pi session (pi::run, pi::start, pi::task, run::start_and_wait and the console terminal). Private: observe it with args { session_id }. Event: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  const raw = createFeed(iii, {
    id: ON_RAW_EVENT_FN,
    source: SOURCE,
    description:
      'Raw Pi AgentSession events (agent_start/end, turn_start/end, message_start/update/end, tool_execution_*, queue_update, compaction_*) of one session, verbatim. Private: observe it with args { session_id }. Event: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  return { agent, raw };
}
