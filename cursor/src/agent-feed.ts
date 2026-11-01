/**
 * Event feeds: `cursor::on-agent-event` (normalized AgentEvent frames) and
 * `cursor::on-raw-event` (raw Cursor ACP / Bridge frames).
 *
 * Each feed is a private function this worker hosts and calls once per frame;
 * its handler returns the frame. A consumer observes a feed with args
 * `{ session_id }` and receives every frame of that session:
 * `{ session_id, event_id, seq, epoch, source, event }`. iii delivers each
 * frame to the observers filed under its session id (`onObserverMatch` by
 * keys); a frame of a session nobody observes is not sent.
 *
 * - `seq` is contiguous per (feed, session_id, epoch) from 0; each feed keeps
 *   its own counter, which advances for every frame, observed or not.
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
 *
 * A frame carries a copy of the event taken when it is emitted. A feed is a
 * private function, and the SDK passes a private function's payload by
 * reference (deep-frozen under NODE_ENV=development), so the copy keeps the
 * caller's own objects, which it may still be mutating, out of the call.
 */

import { randomUUID } from 'node:crypto';
import { CallAction, type IIIClient, ObserveRejection } from 'iii-sdk';

export const MAX_OBSERVERS = 256;
export const MAX_SESSION_ID_LENGTH = 512;
export const ON_AGENT_EVENT_FN = 'cursor::on-agent-event';
export const ON_RAW_EVENT_FN = 'cursor::on-raw-event';
export const FEED_SOURCE = 'cursor';

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
      maxLength: MAX_SESSION_ID_LENGTH,
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
        'The stable `cursor-...` id of a replayed frame, otherwise `<session_id>-<epoch>-<seq:08>`.',
    },
    seq: {
      type: 'integer',
      minimum: 0,
      description: 'Contiguous per (feed, session_id, epoch) from 0.',
    },
    epoch: { type: 'string', description: '`<process uuid>-<generation uuid>`.' },
    source: { type: 'string', description: 'The agent: `cursor`.' },
    event: { description: 'The AgentEvent frame, or the raw Cursor ACP / Bridge frame.' },
  },
  required: ['session_id', 'event_id', 'seq', 'epoch', 'source', 'event'],
};

/**
 * Emit one frame. Resolves `true` when the frame was sent to the session's
 * observers, `false` when nobody observes the session or the call was
 * refused. Callers that track what a consumer has seen (streamed text deltas,
 * `body_streamed`) only advance on `true`, so a frame nobody received is
 * folded into the next delta or the final message body instead of being lost.
 * Never throws.
 */
export type FeedEmit = (
  sessionId: string,
  event: unknown,
  stableEventId?: string,
) => Promise<boolean>;

/** What CursorWorker / RunAccumulator accept; anything but `false` counts as delivered. */
export type Emit = (sessionId: string, event: unknown, stableEventId?: string) => Promise<unknown>;

export type Feed = { emit: FeedEmit };

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
  const sequenceBySession = new Map<string, SequenceState>();
  sequenceStores.add(sequenceBySession);

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

  async function emit(sessionId: string, event: unknown, stableEventId?: string): Promise<boolean> {
    const state = sequenceBySession.get(sessionId) ?? { generation: randomUUID(), sequence: 0 };
    sequenceBySession.set(sessionId, { ...state, sequence: state.sequence + 1 });
    if (!iii.isObserved(opts.id, { input: { session_id: sessionId } })) return false;
    const epoch = `${PROCESS_EPOCH}-${state.generation}`;
    const seq = state.sequence;
    const eventId = stableEventId ?? `${sessionId}-${epoch}-${String(seq).padStart(8, '0')}`;
    try {
      const payload: FeedPayload = {
        session_id: sessionId,
        event_id: eventId,
        seq,
        epoch,
        source: opts.source,
        event: snapshot(event),
      };
      await iii.call({ function_id: opts.id, payload, action: CallAction.Void() });
      return true;
    } catch (error) {
      console.warn(
        `${opts.id}: frame ${eventId} was not sent for ${sessionId}: ${safeError(error)}`,
      );
      return false;
    }
  }

  return { emit };
}

/** The worker's two feeds, each with its own observers and per-session sequence. */
export function createAgentFeeds(iii: IIIClient): { emit: FeedEmit; emitRaw: FeedEmit } {
  const agent = createFeed(iii, {
    id: ON_AGENT_EVENT_FN,
    source: FEED_SOURCE,
    description:
      'Normalized AgentEvent frames of one Cursor session. Private: observe it with args { session_id }. Event: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  const raw = createFeed(iii, {
    id: ON_RAW_EVENT_FN,
    source: FEED_SOURCE,
    description:
      'Raw Cursor ACP / SDK Bridge frames of one session, unchanged. Private: observe it with args { session_id }. Event: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  return { emit: agent.emit, emitRaw: raw.emit };
}

function safeError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.replaceAll(/(?:key|token|secret)_[A-Za-z0-9._-]+/gi, '<redacted>');
}
