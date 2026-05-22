import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AgentMessage } from '../../src/types/agent-message.js';
import * as events from '../../src/turn-orchestrator/events.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import {
  abortSignalKey,
  newRecord,
  type TurnStateRecord,
} from '../../src/turn-orchestrator/state.js';
import { handleSteering, route } from '../../src/turn-orchestrator/states/steering-check.js';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('steering route()', () => {
  it.each([
    [true, true, true, true, 'abort'],
    [true, false, false, false, 'abort'],
    [false, true, true, true, 'steering'],
    [false, true, false, false, 'steering'],
    [false, false, true, true, 'followup'],
    [false, false, true, false, 'followup'],
    [false, false, false, true, 'continue_after_function'],
    [false, false, false, false, 'end_turn'],
  ] as const)(
    'route(%s, %s, %s, %s) -> %s',
    (abort, has_steering, has_followup, has_function_results, expected) => {
      expect(route(abort, has_steering, has_followup, has_function_results)).toBe(expected);
    },
  );
});

function userMessage(text: string): AgentMessage {
  return { role: 'user', content: [{ type: 'text', text }] };
}

function makeIii(
  opts: {
    abort?: boolean;
    steeringItems?: AgentMessage[];
    followupItems?: AgentMessage[];
  } = {},
) {
  const { abort = false, steeringItems = [], followupItems = [] } = opts;
  const drainCalls: Array<{ name: string; session_id: string }> = [];

  const iii = {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      if (req.function_id === 'state::get') {
        const p = req.payload as { key: string };
        if (p.key.endsWith('/abort_signal')) return abort ? true : null;
        return null;
      }
      if (req.function_id === 'session-inbox::drain') {
        const p = req.payload as { name: string; session_id: string };
        drainCalls.push(p);
        if (p.name === 'steering') return { items: steeringItems };
        if (p.name === 'followup') return { items: followupItems };
        return { items: [] };
      }
      if (req.function_id === 'state::update') return { old_value: 0 };
      if (req.function_id === 'stream::set') return null;
      return null;
    }),
  } as unknown as ISdk;

  return { iii, drainCalls };
}

function steeringRec(
  session_id: string,
  overrides: Partial<TurnStateRecord> = {},
): TurnStateRecord {
  const rec = newRecord(session_id);
  rec.state = 'steering_check';
  return { ...rec, ...overrides };
}

describe('handleSteering', () => {
  it('abort: persists aborted assistant, emits turn_end, transitions to tearing_down', async () => {
    const { iii } = makeIii({ abort: true });
    const rec = steeringRec('s1');
    const loadSpy = vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    const saveSpy = vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('tearing_down');
    expect(rec.turn_end_emitted).toBe(true);
    expect(rec.last_assistant?.stop_reason).toBe('aborted');
    expect(loadSpy).toHaveBeenCalledWith(iii, 's1');
    expect(saveSpy).toHaveBeenCalledWith(
      iii,
      's1',
      expect.arrayContaining([expect.objectContaining({ stop_reason: 'aborted' })]),
    );
    expect(emitSpy).toHaveBeenCalledWith(
      iii,
      's1',
      expect.objectContaining({
        type: 'turn_end',
        message: expect.objectContaining({ stop_reason: 'aborted' }),
      }),
    );
  });

  it('abort: skips inbox drains', async () => {
    const { iii, drainCalls } = makeIii({
      abort: true,
      steeringItems: [userMessage('steer')],
      followupItems: [userMessage('follow')],
    });
    const rec = steeringRec('s1');
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(drainCalls).toHaveLength(0);
  });

  it('steering: appends drained messages and transitions to assistant_streaming', async () => {
    const steeringItems = [userMessage('steer-me')];
    const { iii } = makeIii({ steeringItems });
    const rec = steeringRec('s1', {
      function_results: [{ role: 'function_result', content: [] }] as never,
    });
    const loadSpy = vi.spyOn(persistence, 'loadMessages').mockResolvedValue([userMessage('prior')]);
    const saveSpy = vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
    expect(rec.turn_end_emitted).toBe(true);
    expect(saveSpy).toHaveBeenCalledWith(iii, 's1', [userMessage('prior'), ...steeringItems]);
    expect(loadSpy).toHaveBeenCalled();
  });

  it('followup: drains followup when steering queue is empty', async () => {
    const followupItems = [userMessage('follow-up')];
    const { iii, drainCalls } = makeIii({ followupItems });
    const rec = steeringRec('s1');
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    const saveSpy = vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(drainCalls.map((c) => c.name)).toEqual(['steering', 'followup']);
    expect(saveSpy).toHaveBeenCalledWith(iii, 's1', followupItems);
  });

  it('followup: skipped when steering queue has items', async () => {
    const { iii, drainCalls } = makeIii({
      steeringItems: [userMessage('steer')],
      followupItems: [userMessage('follow')],
    });
    const rec = steeringRec('s1');
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(drainCalls.map((c) => c.name)).toEqual(['steering']);
    expect(rec.state).toBe('assistant_streaming');
  });

  it('continue_after_function: clears function_results without reloading messages', async () => {
    const { iii } = makeIii();
    const rec = steeringRec('s1', {
      function_results: [{ role: 'function_result', content: [] }] as never,
      turn_end_emitted: true,
    });
    const loadSpy = vi.spyOn(persistence, 'loadMessages');
    const emitSpy = vi.spyOn(events, 'emit');

    await handleSteering(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
    expect(loadSpy).not.toHaveBeenCalled();
    expect(emitSpy).not.toHaveBeenCalled();
  });

  it('end_turn: emits turn_end once and transitions to tearing_down', async () => {
    const { iii } = makeIii();
    const rec = steeringRec('s1');
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);
    const loadSpy = vi.spyOn(persistence, 'loadMessages');

    await handleSteering(iii, rec);

    expect(rec.state).toBe('tearing_down');
    expect(rec.turn_end_emitted).toBe(true);
    expect(emitSpy).toHaveBeenCalledWith(iii, 's1', expect.objectContaining({ type: 'turn_end' }));
    expect(loadSpy).not.toHaveBeenCalled();
  });

  it('reads abort via state::get on abort_signal key', async () => {
    const { iii } = makeIii({ abort: true });
    const rec = steeringRec('s1');
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(iii.trigger).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'state::get',
        payload: { scope: 'agent', key: abortSignalKey('s1') },
      }),
    );
  });
});
