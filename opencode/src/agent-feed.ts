/**
 * Event feeds: `opencode::on-agent-event` (translated AgentEvent frames) and
 * `opencode::on-raw-event` (verbatim OpenCode JSON events).
 *
 * Each feed is a private function this worker hosts and calls once per frame;
 * its handler returns the frame. A consumer observes a feed with args
 * `{ session_id }` and receives every frame of that session:
 * `{ session_id, event_id, seq, epoch, source, event }`. iii delivers each
 * frame to the observers filed under its session id (`onObserverMatch` by
 * keys); a frame of a session nobody observes is not sent. `seq` is
 * contiguous per (feed, session_id, epoch) from 0 and advances for every
 * frame; `epoch` is a per-process uuid, so a restart starts a new sequence.
 * Frames are ephemeral: nothing is stored or replayed, an observer added late
 * never sees earlier frames. The durable record of a turn is
 * `opencode::status` / `opencode::sessions::list` and the result
 * `opencode::run` returns.
 */

import { randomUUID } from 'node:crypto';
import { CallAction, type IIIClient, ObserveRejection } from 'iii-sdk';

export const MAX_OBSERVERS = 256;
export const MAX_SESSION_ID_LENGTH = 512;
export const ON_AGENT_EVENT_FN = 'opencode::on-agent-event';
export const ON_RAW_EVENT_FN = 'opencode::on-raw-event';
export const FEED_SOURCE = 'opencode';

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
    event_id: { type: 'string', description: '`<session_id>-<epoch>-<seq:08>`, unique per frame.' },
    seq: {
      type: 'integer',
      minimum: 0,
      description: 'Contiguous per (feed, session_id, epoch) from 0.',
    },
    epoch: { type: 'string', description: 'Per-process uuid.' },
    source: { type: 'string', description: 'The agent: `opencode`.' },
    event: { description: 'The AgentEvent frame, or the raw OpenCode JSON event.' },
  },
  required: ['session_id', 'event_id', 'seq', 'epoch', 'source', 'event'],
};

/** Emit one frame; resolves `true` when it was sent to the session's observers, `false` when nobody observes it. */
export type Emit = (session_id: string, event: unknown) => Promise<boolean>;

export type Feed = { emit: Emit };

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

  async function emit(session_id: string, event: unknown): Promise<boolean> {
    const seq = seqBySession.get(session_id) ?? 0;
    seqBySession.set(session_id, seq + 1);
    if (!iii.isObserved(opts.id, { input: { session_id } })) return false;
    const payload: FeedPayload = {
      session_id,
      event_id: `${session_id}-${epoch}-${String(seq).padStart(8, '0')}`,
      seq,
      epoch,
      source: opts.source,
      event,
    };
    try {
      await iii.call({ function_id: opts.id, payload, action: CallAction.Void() });
      return true;
    } catch (err) {
      console.warn(`${opts.id}: frame ${payload.event_id} was not sent: ${String(err)}`);
      return false;
    }
  }

  return { emit };
}

/** The worker's two feeds, each with its own observers and per-session sequence. */
export function createAgentFeeds(iii: IIIClient): { emit: Emit; emitRaw: Emit } {
  const agent = createFeed(iii, {
    id: ON_AGENT_EVENT_FN,
    source: FEED_SOURCE,
    description:
      'Translated AgentEvent frames of one OpenCode session. Private: observe it with args { session_id }. Event: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  const raw = createFeed(iii, {
    id: ON_RAW_EVENT_FN,
    source: FEED_SOURCE,
    description:
      'Raw OpenCode JSON events (verbatim `opencode run --format json` lines) of one session. Private: observe it with args { session_id }. Event: { session_id, event_id, seq, epoch, source, event }. Ephemeral: not stored or replayed.',
  });
  return { emit: agent.emit, emitRaw: raw.emit };
}
