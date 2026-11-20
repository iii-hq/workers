import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ACTIVITY_FN_ID,
  RECOVERY_FN_ID,
  RESTART_REASON,
  settleSessionStatus,
  settleStaleSessions,
  settleStaleSessionsWithRetry,
  startStatusRecovery,
  startupRetry,
  statusRetry,
} from '../src/session-link.js';
import { type FakeIii, fakeIii } from './_helpers/fake-iii.js';

/**
 * When claude-code starts, no run of its own can be live, so a session it
 * created that still says `working` lost its final status (a write that never
 * landed, or a worker that died mid-run). Each namespace has at most one
 * claude-code, and its calls reach only its namespace's session-manager, so
 * every `agent: claude-code` session it can list is its own.
 */

function seed(fake: FakeIii, id: string, status: string, metadata: Record<string, unknown>) {
  fake.state.set(`session-manager/${id}`, { session_id: id, status, metadata });
}

function status(fake: FakeIii, id: string) {
  const meta = fake.state.get(`session-manager/${id}`) as Record<string, unknown>;
  return { status: meta.status, reason: meta.status_reason };
}

describe('settling stale sessions at startup', () => {
  it('marks its own sessions still working as error, with the restart reason', async () => {
    const fake = fakeIii();
    seed(fake, 'stuck-1', 'working', { agent: 'claude-code', parent_session_id: 'p-1' });
    seed(fake, 'stuck-2', 'working', { agent: 'claude-code' });
    expect(await settleStaleSessions(fake.iii, () => false)).toBe(2);
    for (const id of ['stuck-1', 'stuck-2']) {
      expect(status(fake, id)).toEqual({ status: 'error', reason: RESTART_REASON });
    }
  });

  it('leaves finished sessions, other agents and its own live runs alone', async () => {
    const fake = fakeIii();
    seed(fake, 'done-1', 'done', { agent: 'claude-code' });
    seed(fake, 'pi-1', 'working', { agent: 'pi' });
    seed(fake, 'chat-1', 'working', {});
    seed(fake, 'live-1', 'working', { agent: 'claude-code' });
    expect(await settleStaleSessions(fake.iii, (id) => id === 'live-1')).toBe(0);
    expect(status(fake, 'done-1').status).toBe('done');
    expect(status(fake, 'pi-1').status).toBe('working');
    expect(status(fake, 'chat-1').status).toBe('working');
    expect(status(fake, 'live-1').status).toBe('working');
    const writes = fake.calls.filter((c) => c.function_id === 'session::set-status');
    expect(writes).toEqual([]);
  });

  it('reads every page before writing, so no session is skipped', async () => {
    // Writing while paging a status-filtered list would move later sessions
    // out from under the cursor.
    const fake = fakeIii();
    const ids = Array.from({ length: 230 }, (_, i) => `many-${i}`);
    for (const id of ids) seed(fake, id, 'working', { agent: 'claude-code' });
    expect(await settleStaleSessions(fake.iii, () => false)).toBe(230);
    expect(ids.filter((id) => status(fake, id).status !== 'error')).toEqual([]);
    const lists = fake.calls.filter((c) => c.function_id === 'session::list');
    expect(lists.length).toBeGreaterThan(1);
    for (const c of lists) {
      expect(c.payload).toMatchObject({ status: 'working', metadata: { agent: 'claude-code' } });
    }
  });

  it('a session-manager that does not answer costs nothing but a warning', async () => {
    const fake = fakeIii();
    (fake.iii as { call: unknown }).call = async () => {
      throw new Error('function_not_found');
    };
    // null, not 0: a listing that failed is not a scan that found nothing.
    await expect(settleStaleSessions(fake.iii, () => false)).resolves.toBeNull();
  });

  it('retries a failed listing in the background until one succeeds', async () => {
    const fake = fakeIii();
    seed(fake, 'late-1', 'working', { agent: 'claude-code' });
    const real = fake.iii.call.bind(fake.iii);
    let lists = 0;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'session::list' && ++lists <= 2)
        throw new Error('function_not_found');
      return real(req as never);
    };
    const saved = startupRetry.delaysMs;
    startupRetry.delaysMs = [0, 0, 0];
    try {
      expect(await settleStaleSessionsWithRetry(fake.iii, () => false)).toBe(1);
    } finally {
      startupRetry.delaysMs = saved;
    }
    expect(lists).toBe(3);
    expect(status(fake, 'late-1').status).toBe('error');
  });

  it('gives up after its bounded retries', async () => {
    const fake = fakeIii();
    let lists = 0;
    (fake.iii as { call: unknown }).call = async () => {
      lists++;
      throw new Error('function_not_found');
    };
    const saved = startupRetry.delaysMs;
    startupRetry.delaysMs = [0, 0];
    try {
      expect(await settleStaleSessionsWithRetry(fake.iii, () => false)).toBeNull();
    } finally {
      startupRetry.delaysMs = saved;
    }
    expect(lists).toBe(3);
  });
});

/**
 * Recovery that waits for nothing but session-manager itself: claude-code
 * observes engine::on-functions-changed, and when the event lists
 * `session::set-status` (session-manager registered, e.g. after it restarted)
 * it writes every status still pending and runs a startup scan that has not
 * succeeded yet. No timer and no other status write is needed.
 */
describe('recovery when session-manager registers', () => {
  const saved = { status: statusRetry.delaysMs, startup: startupRetry.delaysMs };
  beforeEach(() => {
    statusRetry.delaysMs = [0, 0];
    // Long enough that the timer retry never fires during a test.
    startupRetry.delaysMs = [60_000];
  });
  afterEach(() => {
    statusRetry.delaysMs = saved.status;
    startupRetry.delaysMs = saved.startup;
  });

  function functionsChanged(ids: string[]) {
    return {
      observer_id: 'o',
      activation_id: 'a',
      function_id: 'engine::on-functions-changed',
      namespace: 'default',
      invocation_id: 'i',
      status: 'success',
      started_at_ms: 0,
      duration_ms: 0,
      input: {
        event: 'functions_changed',
        functions: ids.map((function_id) => ({
          function_id,
          namespace: 'default',
          worker_name: 'session-manager',
          observer_count: 0,
        })),
      },
    };
  }

  it('observes engine::on-functions-changed with its input', async () => {
    const fake = fakeIii();
    startStatusRecovery(
      fake.iii,
      () => false,
      () => {},
    );
    expect(fake.functions.has(RECOVERY_FN_ID)).toBe(true);
    expect(fake.observed).toContainEqual(
      expect.objectContaining({
        source: expect.objectContaining({
          function_id: 'engine::on-functions-changed',
          capture: ['input'],
        }),
        target: RECOVERY_FN_ID,
      }),
    );
  });

  it('writes a status that exhausted its retries when session-manager registers, with no other write', async () => {
    const fake = fakeIii();
    startStatusRecovery(
      fake.iii,
      () => false,
      () => {},
    );
    const real = fake.iii.call.bind(fake.iii);
    let down = true;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'session::set-status' && down) throw new Error('function_not_found');
      return real(req as never);
    };
    seed(fake, 'lost-sm', 'working', { agent: 'claude-code' });
    expect(await settleSessionStatus(fake.iii, 'lost-sm', 'done')).toBe(false);
    // An unrelated registration changes nothing while session-manager is away.
    await fake.registered.get(RECOVERY_FN_ID)?.(functionsChanged(['other::fn']));
    expect(status(fake, 'lost-sm').status).toBe('working');
    down = false;
    await fake.registered.get(RECOVERY_FN_ID)?.(functionsChanged(['session::set-status']));
    await vi.waitFor(() => expect(status(fake, 'lost-sm').status).toBe('done'));
  });

  it('runs a startup scan whose listing failed once session-manager registers', async () => {
    const fake = fakeIii();
    seed(fake, 'stale-sm', 'working', { agent: 'claude-code' });
    const real = fake.iii.call.bind(fake.iii);
    let down = true;
    let lists = 0;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'session::list') {
        lists++;
        if (down) throw new Error('function_not_found');
      }
      return real(req as never);
    };
    let scanned = 0;
    startStatusRecovery(
      fake.iii,
      () => false,
      () => {
        scanned++;
      },
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(status(fake, 'stale-sm').status).toBe('working');
    expect(scanned).toBe(0);
    down = false;
    await fake.registered.get(RECOVERY_FN_ID)?.(functionsChanged(['session::set-status']));
    await vi.waitFor(() =>
      expect(status(fake, 'stale-sm')).toEqual({ status: 'error', reason: RESTART_REASON }),
    );
    expect(scanned).toBe(1);
    // Scanned once: a later registration does not scan again.
    await fake.registered.get(RECOVERY_FN_ID)?.(functionsChanged(['session::set-status']));
    expect(lists).toBe(2);
  });
});

/**
 * session-manager can stay registered while its store fails, then recover
 * with no registration event. Its own commit events show the recovery: while
 * anything is pending, claude-code observes session::on-status-changed and
 * session::on-message-added (any caller's commit), and disposes them once
 * nothing is pending.
 */
describe('recovery on session-manager activity', () => {
  const saved = { status: statusRetry.delaysMs, startup: startupRetry.delaysMs };
  beforeEach(() => {
    statusRetry.delaysMs = [0, 0];
    startupRetry.delaysMs = [60_000];
  });
  afterEach(() => {
    statusRetry.delaysMs = saved.status;
    startupRetry.delaysMs = saved.startup;
  });

  const ACTIVITY = ['session::on-status-changed', 'session::on-message-added'];

  /** Observers of session-manager's commit events, and whether each is disposed. */
  function activityWatch(fake: FakeIii) {
    return fake.observed
      .map((o, i) => ({ o, id: `observer-${i + 1}` }))
      .filter(({ o }) =>
        ACTIVITY.includes((o.source as { function_id?: string }).function_id ?? ''),
      )
      .map(({ o, id }) => ({
        source: (o.source as { function_id: string }).function_id,
        target: o.target,
        live: !fake.disposed.includes(id),
      }));
  }

  const activity = {
    observer_id: 'o',
    activation_id: 'a',
    function_id: 'session::on-message-added',
    namespace: 'default',
    invocation_id: 'i',
    status: 'success',
    started_at_ms: 0,
    duration_ms: 0,
  };

  it('watches nothing once the startup scan succeeded and nothing is pending', async () => {
    const fake = fakeIii();
    let scanned = false;
    startStatusRecovery(
      fake.iii,
      () => false,
      () => {
        scanned = true;
      },
    );
    await vi.waitFor(() => expect(scanned).toBe(true));
    expect(activityWatch(fake).filter((w) => w.live)).toEqual([]);
  });

  it('writes a lost status once the store commits anything again, then stops watching', async () => {
    const fake = fakeIii();
    let scanned = false;
    startStatusRecovery(
      fake.iii,
      () => false,
      () => {
        scanned = true;
      },
    );
    await vi.waitFor(() => expect(scanned).toBe(true));
    // The store fails while session-manager stays registered.
    const real = fake.iii.call.bind(fake.iii);
    let storeDown = true;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'session::set-status' && storeDown)
        throw new Error('store unavailable');
      return real(req as never);
    };
    seed(fake, 'store-1', 'working', { agent: 'claude-code' });
    expect(await settleSessionStatus(fake.iii, 'store-1', 'done')).toBe(false);
    await vi.waitFor(() =>
      expect(
        activityWatch(fake)
          .filter((w) => w.live)
          .map((w) => w.source)
          .sort(),
      ).toEqual([...ACTIVITY].sort()),
    );
    // The store recovers silently; another worker's commit is the signal.
    storeDown = false;
    await fake.registered.get(ACTIVITY_FN_ID)?.(activity);
    await vi.waitFor(() => expect(status(fake, 'store-1').status).toBe('done'));
    await vi.waitFor(() => expect(activityWatch(fake).filter((w) => w.live)).toEqual([]));
  });

  it('runs a startup scan that could not list once the store commits anything', async () => {
    const fake = fakeIii();
    seed(fake, 'store-2', 'working', { agent: 'claude-code' });
    const real = fake.iii.call.bind(fake.iii);
    let storeDown = true;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'session::list' && storeDown) throw new Error('store unavailable');
      return real(req as never);
    };
    let scanned = 0;
    startStatusRecovery(
      fake.iii,
      () => false,
      () => {
        scanned++;
      },
    );
    await vi.waitFor(() => expect(activityWatch(fake).filter((w) => w.live).length).toBe(2));
    storeDown = false;
    await fake.registered.get(ACTIVITY_FN_ID)?.(activity);
    await vi.waitFor(() =>
      expect(status(fake, 'store-2')).toEqual({ status: 'error', reason: RESTART_REASON }),
    );
    expect(scanned).toBe(1);
    await vi.waitFor(() => expect(activityWatch(fake).filter((w) => w.live)).toEqual([]));
  });
});
