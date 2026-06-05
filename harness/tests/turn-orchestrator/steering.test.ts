import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as events from '../../src/turn-orchestrator/events.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';
import { newRecord, type TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import { handleSteering } from '../../src/turn-orchestrator/steering-check/process.js';

afterEach(() => {
  vi.restoreAllMocks();
});

function makeIii() {
  const iii = {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      if (req.function_id === 'state::get') return null;
      if (req.function_id === 'state::update') return { old_value: 0 };
      if (req.function_id === 'stream::set') return null;
      return null;
    }),
  } as unknown as ISdk;

  return { iii };
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
