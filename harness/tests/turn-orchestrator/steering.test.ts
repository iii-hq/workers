import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AgentMessage } from '../../src/types/agent-message.js';
import * as events from '../../src/turn-orchestrator/events.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';
import { newRecord, type TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import { handleSteering } from '../../src/turn-orchestrator/steering-check/process.js';
import { route } from '../../src/turn-orchestrator/steering-check/run.js';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('steering route()', () => {
  it.each([
    [true, true, true, 'steering'],
    [true, false, false, 'steering'],
    [false, true, true, 'followup'],
    [false, true, false, 'followup'],
    [false, false, true, 'continue_after_function'],
    [false, false, false, 'end_turn'],
  ] as const)('route(%s, %s, %s) -> %s', (has_steering, has_followup, has_function_results, expected) => {
    expect(route(has_steering, has_followup, has_function_results)).toBe(expected);
  });
});

function userMessage(text: string): AgentMessage {
  return { role: 'user', content: [{ type: 'text', text }] };
}

function makeIii(opts: { steeringItems?: AgentMessage[]; followupItems?: AgentMessage[] } = {}) {
  const { steeringItems = [], followupItems = [] } = opts;
  const drainCalls: Array<{ name: string; session_id: string }> = [];

  const iii = {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      if (req.function_id === 'state::get') return null;
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
  it('steering: appends drained messages and transitions to assistant_streaming', async () => {
    const steeringItems = [userMessage('steer-me')];
    const { iii } = makeIii({ steeringItems });
    const rec = steeringRec('s1', {
      function_results: [{ role: 'function_result', content: [] }] as never,
    });
    const store = installMockTurnStore();
    const appendSpy = store.appendMessages;
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
    expect(rec.turn_end_emitted).toBe(true);
    expect(appendSpy).toHaveBeenCalledWith('s1', steeringItems);
    expect(store.loadMessages).not.toHaveBeenCalled();
  });

  it('followup: drains followup when steering queue is empty', async () => {
    const followupItems = [userMessage('follow-up')];
    const { iii, drainCalls } = makeIii({ followupItems });
    const rec = steeringRec('s1');
    const store = installMockTurnStore();
    const appendSpy = store.appendMessages;
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(drainCalls.map((c) => c.name)).toEqual(['steering', 'followup']);
    expect(appendSpy).toHaveBeenCalledWith('s1', followupItems);
  });

  it('followup: skipped when steering queue has items', async () => {
    const { iii, drainCalls } = makeIii({
      steeringItems: [userMessage('steer')],
      followupItems: [userMessage('follow')],
    });
    const rec = steeringRec('s1');
    installMockTurnStore();
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
    const store = installMockTurnStore();
    const emitSpy = vi.spyOn(events, 'emit');

    await handleSteering(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
    expect(store.loadMessages).not.toHaveBeenCalled();
    expect(emitSpy).not.toHaveBeenCalled();
  });

  it('end_turn: emits turn_end then finishes the session (agent_end + stopped)', async () => {
    const { iii } = makeIii();
    const rec = steeringRec('s1');
    const store = installMockTurnStore();
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('stopped');
    expect(rec.turn_end_emitted).toBe(true);
    expect(emitSpy).toHaveBeenCalledWith(iii, 's1', expect.objectContaining({ type: 'turn_end' }));
    expect(emitSpy).toHaveBeenCalledWith(iii, 's1', expect.objectContaining({ type: 'agent_end' }));
    // agent_end is a signal: finishSession no longer reloads the transcript.
    expect(store.loadMessages).not.toHaveBeenCalled();
  });

  it('caps at max_turns: emits a max_turns assistant + message_complete + turn_end and tears down instead of continuing', async () => {
    const { iii } = makeIii();
    const rec = steeringRec('s1', {
      max_turns: 2,
      turn_count: 2,
      function_results: [{ role: 'function_result', content: [] }] as never,
    });
    const store = installMockTurnStore();
    const appendSpy = store.appendMessages;
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('stopped');
    expect(rec.turn_end_emitted).toBe(true);
    expect(rec.last_assistant?.content[0]).toEqual(
      expect.objectContaining({ type: 'text', text: expect.stringContaining('max_turns') }),
    );
    expect(emitSpy).toHaveBeenCalledWith(
      iii,
      's1',
      expect.objectContaining({ type: 'message_complete' }),
    );
    expect(emitSpy).toHaveBeenCalledWith(iii, 's1', expect.objectContaining({ type: 'turn_end' }));
    // max_turns teardown appends the synthetic notice and finishes without a
    // transcript reload (agent_end is a signal).
    expect(store.loadMessages).not.toHaveBeenCalled();
    expect(appendSpy).toHaveBeenCalledWith('s1', [
      expect.objectContaining({
        content: expect.arrayContaining([
          expect.objectContaining({ text: expect.stringContaining('max_turns') }),
        ]),
      }),
    ]);
  });

  it('caps at max_turns via steering route: tears down instead of continuing to assistant_streaming', async () => {
    const { iii } = makeIii({ steeringItems: [userMessage('steer-me')] });
    const rec = steeringRec('s1', {
      max_turns: 3,
      turn_count: 3,
    });
    installMockTurnStore({ loadMessages: vi.fn(async () => []) });
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('stopped');
    expect(rec.turn_end_emitted).toBe(true);
    expect(rec.last_assistant?.content[0]).toEqual(
      expect.objectContaining({ text: expect.stringContaining('max_turns') }),
    );
  });

  it('continues to assistant_streaming when under max_turns (continue_after_function route)', async () => {
    const { iii } = makeIii();
    const rec = steeringRec('s1', {
      max_turns: 5,
      turn_count: 2,
      function_results: [{ role: 'function_result', content: [] }] as never,
    });
    installMockTurnStore();
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleSteering(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
  });
});
