import { type Call, type IIIClient, type Observer, ObserveRejection } from 'iii-sdk';
import { z } from 'zod';
import type { AgentRow } from './agents.js';
import { replyLease, type ReplyPermission } from './configuration.js';
import { jsonSchema } from './run.js';
import { readTranscript, Reply } from './terminal/external-transcript.js';

const Args = z.strictObject({ claude_session_id: z.guid() });
const Event = z.object({ claude_session_id: z.guid(), reply: Reply });
type ReplyEvent = z.infer<typeof Event>;
const ON_REPLY = 'claude::agents::on-reply';
type Row = (id: string) => Promise<{ row: AgentRow; env: Record<string, string> }>;
type Schedule = (poll: () => Promise<void>) => () => void;
const schedulePoll: Schedule = (poll) => {
  const timer = setTimeout(poll, 2_000);
  timer.unref();
  return () => clearTimeout(timer);
};

function sessionId(args: unknown) {
  const parsed = Args.safeParse(args);
  if (!parsed.success) throw new Error('invalid Claude session id');
  return parsed.data.claude_session_id;
}

type Watch = { epoch: number; cursor: string; active: boolean };
type Recovery = { epoch: number; env: Record<string, string>; cursor?: string };
type Group = {
  watches: Map<string, Watch>;
  tail: Promise<void>;
  recovery?: Recovery;
  cancel?: () => void;
};

/** Each observer owns its admission boundary. Readers with the same boundary
 * share a poll; a later admission never advances an earlier observer. */
export function registerExternalReplies(
  iii: IIIClient,
  permission: ReplyPermission,
  rowFor: Row,
  currentEnv: () => Record<string, string>,
  schedule: Schedule = schedulePoll,
) {
  // The pinned SDK records invocation payloads independently of observable
  // and trace_hidden. There is no function-level payload switch in 0.27.
  process.env.III_DISABLE_TRACE_PAYLOADS = '1';
  const groups = new Map<string, Group>();
  let count = 0;
  const acquire = async () => replyLease(permission, await permission.confirm());

  iii.registerFunction(
    'claude::agents::get',
    async (args: unknown) => {
      const lease = await acquire();
      const id = sessionId(args);
      const { row, env } = await lease.read(() => rowFor(id));
      const { replies } = await lease.read(() => readTranscript(iii, env, id, 'last'));
      lease.check();
      return {
        claude_session_id: id,
        name: row.name ?? null,
        cwd: row.cwd ?? null,
        status: row.status,
        last_reply: replies[0] ?? null,
      };
    },
    {
      description: 'Read a listed terminal session and its last active-branch assistant reply.',
      observable: false,
      metadata: { trace_hidden: true },
      request_format: jsonSchema(Args),
      response_format: jsonSchema(
        z.object({
          claude_session_id: z.guid(),
          name: z.string().nullable(),
          cwd: z.string().nullable(),
          status: z.string(),
          last_reply: Reply.nullable(),
        }),
      ),
    },
  );

  // iii-sdk 0.27 queues invocations made on a closed socket for reconnect.
  // Its public connection state changes on the close event, after CLOSING.
  // Use its synchronous socket check and fail closed if the SDK lacks it.
  function requireOpenSocket() {
    if (!('isOpen' in iii && typeof iii.isOpen === 'function' && iii.isOpen())) {
      throw new Error('reply delivery unavailable');
    }
  }

  function matchesWatch(obs: Observer, call: Call<ReplyEvent, ReplyEvent>) {
    const id = call.input.claude_session_id;
    return (
      obs.args.claude_session_id === id &&
      obs.id === call.metadata?.for_observer &&
      permission.epoch !== null &&
      permission.epoch === call.metadata?.epoch &&
      groups.get(id)?.watches.get(obs.id)?.active === true
    );
  }

  function serialize<T>(g: Group, work: () => Promise<T>) {
    const result = g.tail.then(work);
    g.tail = result.then(
      () => {},
      () => {},
    );
    return result;
  }

  function remove(id: string, observer: string, watch?: Watch) {
    const g = groups.get(id);
    if (!g || (watch && g.watches.get(observer) !== watch) || !g.watches.delete(observer)) return;
    count--;
    if (!g.watches.size) {
      g.cancel?.();
      // Keep the lock until a read already on the host has actually returned.
      void g.tail.then(() => {
        if (!g.watches.size && groups.get(id) === g) groups.delete(id);
      });
      g.cancel = undefined;
    }
  }

  async function recoverWatches(epoch: number) {
    const lease = replyLease(permission, epoch);
    const needsBaseline = (w: Watch) => w.active && w.epoch !== epoch;
    const checkpoints = [];
    for (const [id, g] of groups) {
      if (g.recovery?.epoch === epoch || ![...g.watches.values()].some(needsBaseline)) continue;
      const recovery: Recovery = { epoch, env: currentEnv() };
      g.recovery = recovery;
      // Capture only a cursor for a previously admitted session, before slow
      // inventory can move the cutoff. This does not authorize any delivery.
      const checkpoint = lease
        .read(() => readTranscript(iii, recovery.env, id, 'baseline'))
        .then(
          ({ cursor }) => {
            // A validated viewer may already have captured an earlier tip.
            recovery.cursor ??= cursor;
          },
          () => {},
        );
      checkpoints.push(checkpoint);
      void serialize(g, async () => {
        if (![...g.watches.values()].some(needsBaseline)) return;
        const { env } = await lease.read(() => rowFor(id));
        await checkpoint;
        lease.check();
        if (env !== recovery.env || !recovery.cursor) return;
        for (const w of g.watches.values()) {
          if (!needsBaseline(w)) continue;
          w.cursor = recovery.cursor;
          w.epoch = epoch;
        }
      }).catch(() => {
        // If inventory failed, an authorized poll can still use this checkpoint.
      });
    }
    await Promise.all(checkpoints);
  }

  permission.onConfirmedRecovery(recoverWatches, ({ id, epoch, env, cursor }) => {
    const recovery = groups.get(id)?.recovery;
    if (recovery?.epoch === epoch && recovery.env === env) recovery.cursor ??= cursor;
  });

  async function poll(id: string, g: Group, lease: ReturnType<typeof replyLease>) {
    const { epoch } = lease;
    if (![...g.watches.values()].some((w) => w.active)) return;
    const { env } = await lease.read(() => rowFor(id));
    const recovery = g.recovery;
    const checkpoint =
      recovery?.epoch === epoch && recovery.env === env ? recovery.cursor : undefined;
    const batches = new Map<string, Array<[string, Watch]>>();
    for (const entry of g.watches) {
      const w = entry[1];
      if (!w.active) continue;
      const cursor = w.epoch === epoch ? w.cursor : (checkpoint ?? '');
      const batch = batches.get(cursor) ?? [];
      batch.push(entry);
      batches.set(cursor, batch);
    }
    for (const [cursor, batch] of batches) {
      if (!batch.some(([key, w]) => g.watches.get(key) === w)) continue;
      let next: Awaited<ReturnType<typeof readTranscript>>;
      try {
        next = await lease.read(() =>
          readTranscript(iii, env, id, cursor ? 'since' : 'baseline', cursor),
        );
      } catch {
        // Retry this cursor without holding up readers at a different boundary.
        lease.check(); // A lost grant, unlike a failed page, ends the whole poll.
        continue;
      }
      for (const [key, w] of batch) {
        for (const reply of next.replies) {
          lease.check();
          if (g.watches.get(key) !== w) break;
          requireOpenSocket();
          await iii.call({
            function_id: ON_REPLY,
            metadata: { for_observer: key, epoch },
            payload: { claude_session_id: id, reply },
          });
        }
        w.cursor = next.cursor;
        w.epoch = epoch;
      }
    }
  }

  function arm(id: string, g: Group) {
    if (g.cancel || !Array.from(g.watches.values()).some((w) => w.active)) return;
    const cancel = schedule(async () => {
      try {
        // Confirmation can queue a recovery inventory check on this group.
        const lease = await acquire();
        await serialize(g, () => poll(id, g, lease));
      } catch {
        // No cursor advance on a failed read. Never log host or delivery errors.
      } finally {
        if (g.cancel === cancel) {
          g.cancel = undefined;
          arm(id, g);
        }
      }
    });
    g.cancel = cancel;
  }

  iii
    .registerFunction<ReplyEvent, ReplyEvent>(ON_REPLY, (event) => event, {
      private: true,
      metadata: { trace_hidden: true },
      description:
        'Observe new active-branch replies after a per-session transcript baseline. Targets receive captured call fields (output is the reply event); the private poller receives no result.',
      args_format: jsonSchema(Args),
      request_format: jsonSchema(Event),
      response_format: jsonSchema(Event),
    })
    .onObserverAdded(async (obs) => {
      const lease = await acquire();
      const id = sessionId(obs.args);
      if (count >= 256) throw new ObserveRejection('OBSERVER_LIMIT', 'reply observer limit');
      const g: Group = groups.get(id) ?? { watches: new Map(), tail: Promise.resolve() };
      const w: Watch = { epoch: lease.epoch, cursor: '', active: false };
      g.watches.set(obs.id, w);
      groups.set(id, g);
      count++;
      try {
        await serialize(g, async () => {
          const { env } = await lease.read(() => rowFor(id));
          const next = await lease.read(() => readTranscript(iii, env, id, 'baseline'));
          w.cursor = next.cursor;
        });
        lease.check();
        void obs.accepted().then(
          () => {
            if (g.watches.get(obs.id) !== w) return;
            w.active = true;
            arm(id, g);
          },
          () => remove(id, obs.id, w),
        );
      } catch (err) {
        remove(id, obs.id, w);
        throw err;
      }
    })
    .onObserverRemoved((obs) => remove(sessionId(obs.args), obs.id))
    .onObservationMatch(matchesWatch)
    .onObservationDeliver(async (call, matched) => {
      // Do not leave reply text in the SDK's deferred observation reporter.
      // A match is a snapshot, not authorization to dispatch later.
      for (const observer of matched) {
        if (!observer.matches(call) || !matchesWatch(observer, call)) continue;
        const payload = Object.fromEntries(
          observer.capture.filter((key) => call[key] !== undefined).map((key) => [key, call[key]]),
        );
        requireOpenSocket();
        await iii.call({ observer, payload, action: { type: 'void' } });
      }
    });
}
