import { describe, expect, it } from 'vitest';
import { RESTART_REASON, settleStaleSessions } from '../src/session-link.js';
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
    await expect(settleStaleSessions(fake.iii, () => false)).resolves.toBe(0);
  });
});
