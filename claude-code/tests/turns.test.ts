import { describe, expect, it } from 'vitest';
import { RESTART_REASON, startupRetry } from '../src/session-link.js';
import { settleStaleTurns } from '../src/turns.js';
import { type FakeIii, fakeIii } from './_helpers/fake-iii.js';

function seed(fake: FakeIii, session_id: string, status: string, turn_id?: string) {
  fake.state.set(`claude_sessions/${session_id}`, {
    session_id,
    claude_session_id: `cs-${session_id}`,
    cwd: '',
    model: '',
    status,
    turns: 1,
    total_cost_usd: 0,
    usage: null,
    updated_at_ms: 1,
    ...(turn_id ? { turn_id } : {}),
  });
}

describe('settling turns a restart left working', () => {
  it('marks a working run error and settles its turn', async () => {
    const fake = fakeIii();
    seed(fake, 'dead-1', 'working', 't-dead');
    seed(fake, 'done-1', 'done', 't-done');
    expect(await settleStaleTurns(fake.iii, () => false)).toBe(1);
    expect(fake.state.get('claude_sessions/dead-1')).toMatchObject({ status: 'error' });
    expect(fake.state.get('claude_turns/dead-1')).toMatchObject({
      id: 'dead-1',
      source: 'boot',
      status: 'error',
      stop_reason: 'worker_restart',
      turn_id: 't-dead',
      claude_session_id: 'cs-dead-1',
      error: RESTART_REASON,
    });
    expect(fake.state.has('claude_turns/done-1')).toBe(false);
  });

  it('spares a session that ran since start', async () => {
    const fake = fakeIii();
    seed(fake, 'live-1', 'working', 't-live');
    expect(await settleStaleTurns(fake.iii, (id) => id === 'live-1')).toBe(0);
    expect(fake.state.get('claude_sessions/live-1')).toMatchObject({ status: 'working' });
    expect(fake.state.has('claude_turns/live-1')).toBe(false);
  });

  it('leaves a run that ended after the listing untouched', async () => {
    const fake = fakeIii();
    seed(fake, 'ended-1', 'working', 't-ended');
    const real = fake.iii.call.bind(fake.iii);
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      const out = await real(req as never);
      // The run saves `done` between the listing and the re-read.
      if (req.function_id === 'state::list') seed(fake, 'ended-1', 'done', 't-ended');
      return out;
    };
    expect(await settleStaleTurns(fake.iii, () => false)).toBe(0);
    expect(fake.state.get('claude_sessions/ended-1')).toMatchObject({ status: 'done' });
    expect(fake.state.has('claude_turns/ended-1')).toBe(false);
  });

  it('retries a failed listing', async () => {
    const fake = fakeIii();
    seed(fake, 'late-1', 'working', 't-late');
    const real = fake.iii.call.bind(fake.iii);
    let lists = 0;
    (fake.iii as { call: unknown }).call = async (req: { function_id: string }) => {
      if (req.function_id === 'state::list' && ++lists <= 2) throw new Error('state down');
      return real(req as never);
    };
    const saved = startupRetry.delaysMs;
    startupRetry.delaysMs = [0, 0, 0];
    try {
      expect(await settleStaleTurns(fake.iii, () => false)).toBe(1);
    } finally {
      startupRetry.delaysMs = saved;
    }
    expect(lists).toBe(3);
    expect(fake.state.get('claude_turns/late-1')).toMatchObject({ turn_id: 't-late' });
  });

  it('gives up after its bounded retries', async () => {
    const fake = fakeIii();
    (fake.iii as { call: unknown }).call = async () => {
      throw new Error('state down');
    };
    const saved = startupRetry.delaysMs;
    startupRetry.delaysMs = [0, 0];
    try {
      expect(await settleStaleTurns(fake.iii, () => false)).toBeNull();
    } finally {
      startupRetry.delaysMs = saved;
    }
  });
});
