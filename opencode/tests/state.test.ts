import { describe, expect, it } from 'vitest';
import type { ISdk } from 'iii-sdk';
import { listSessions, loadSession, saveSession } from '../src/state.js';
import type { SessionRecord } from '../src/types.js';
import { fakeIii } from './_helpers/fake-iii.js';

function record(session_id: string): SessionRecord {
  return {
    session_id,
    opencode_session_id: `ses_${session_id}`,
    cwd: '/repo',
    model: '',
    status: 'done',
    turns: 1,
    total_cost_usd: 0.01,
    usage: null,
    updated_at_ms: 1,
  };
}

describe('session state', () => {
  it('round-trips a record through scope opencode_sessions', async () => {
    const fake = fakeIii();
    await saveSession(fake.iii, record('s1'));
    const set = fake.calls.find((c) => c.function_id === 'state::set');
    expect(set?.payload).toMatchObject({ scope: 'opencode_sessions', key: 's1' });
    await expect(loadSession(fake.iii, 's1')).resolves.toMatchObject({
      session_id: 's1',
      opencode_session_id: 'ses_s1',
    });
  });

  it('returns null for unknown sessions', async () => {
    const fake = fakeIii();
    await expect(loadSession(fake.iii, 'missing')).resolves.toBeNull();
  });

  it('handles state::get returning the value directly, without an envelope', async () => {
    const iii = {
      trigger: async (req: { function_id: string }) =>
        req.function_id === 'state::get' ? record('s1') : null,
    } as unknown as ISdk;
    await expect(loadSession(iii, 's1')).resolves.toMatchObject({ session_id: 's1' });
  });

  it('lists every record and tolerates a non-array reply', async () => {
    const fake = fakeIii();
    await saveSession(fake.iii, record('s1'));
    await saveSession(fake.iii, record('s2'));
    const sessions = await listSessions(fake.iii);
    expect(sessions.map((s) => s.session_id).sort()).toEqual(['s1', 's2']);
    const iii = { trigger: async () => null } as unknown as ISdk;
    await expect(listSessions(iii)).resolves.toEqual([]);
  });
});
