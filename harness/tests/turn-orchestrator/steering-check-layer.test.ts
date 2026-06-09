import { describe, expect, it, vi } from 'vitest';
import {
  applySteeringCheckOutcome,
  processSteeringCheck,
  route,
} from '../../src/turn-orchestrator/steering-check/run.js';
import type { SteeringCheckPorts } from '../../src/turn-orchestrator/steering-check/ports.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

function stubPorts(overrides: Partial<SteeringCheckPorts> = {}): SteeringCheckPorts {
  return {
    loadMessages: vi.fn(async () => []),
    appendMessages: vi.fn(async () => {}),
    checkpoint: vi.fn(async () => {}),
    loadRunRequest: vi.fn(async () => ({
      provider: 'openai',
      model: 'gpt-4',
      mode: null,
      system_prompt: '',
      function_schemas: [],
    })),
    saveRunRequest: vi.fn(async () => {}),
    emitTurnEnd: vi.fn(async () => {}),
    finishSession: vi.fn(async (rec) => {
      rec.state = 'stopped';
    }),
    emit: vi.fn(async () => {}),
    ...overrides,
  };
}

describe('route', () => {
  it.each([
    [true, 'continue_after_function'],
    [false, 'end_turn'],
  ] as const)('route(%s) -> %s', (has_function_results, expected) => {
    expect(route(has_function_results)).toBe(expected);
  });
});

describe('processSteeringCheck', () => {
  it('returns continue_after_function when function_results present', async () => {
    const ports = stubPorts();
    const rec = {
      ...newRecord('s1'),
      state: 'steering_check' as const,
      function_results: [{ role: 'function_result', content: [] }] as never,
    };

    const outcome = await processSteeringCheck(ports, rec);

    expect(outcome).toEqual({ kind: 'continue_after_function' });
  });

  it('returns max_turns_reached when cap hit on continue path', async () => {
    const ports = stubPorts();
    const rec = {
      ...newRecord('s1'),
      state: 'steering_check' as const,
      max_turns: 2,
      turn_count: 2,
      function_results: [{ role: 'function_result', content: [] }] as never,
    };

    const outcome = await processSteeringCheck(ports, rec);

    expect(outcome).toEqual({ kind: 'max_turns_reached' });
  });

  it('returns end_turn when no function results', async () => {
    const ports = stubPorts();
    const rec = { ...newRecord('s1'), state: 'steering_check' as const };

    const outcome = await processSteeringCheck(ports, rec);

    expect(outcome).toEqual({ kind: 'end_turn' });
  });
});

describe('applySteeringCheckOutcome', () => {
  it('continue_after_function: transitions without reloading messages', async () => {
    const loadMessages = vi.fn(async () => []);
    const emitTurnEnd = vi.fn(async () => {});
    const ports = stubPorts({ loadMessages, emitTurnEnd });
    const rec = {
      ...newRecord('s1'),
      state: 'steering_check' as const,
      function_results: [{ role: 'function_result', content: [] }] as never,
      turn_end_emitted: true,
    };

    await applySteeringCheckOutcome(ports, rec, { kind: 'continue_after_function' });

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
    expect(loadMessages).not.toHaveBeenCalled();
    expect(emitTurnEnd).not.toHaveBeenCalled();
  });

  it('end_turn: emits turn_end then routes to the finishing step', async () => {
    const emitTurnEnd = vi.fn(async () => {});
    const finishSession = vi.fn(async (rec) => {
      rec.state = 'stopped';
    });
    const ports = stubPorts({ emitTurnEnd, finishSession });
    const rec = { ...newRecord('s1'), state: 'steering_check' as const };

    await applySteeringCheckOutcome(ports, rec, { kind: 'end_turn' });

    // agent_end is deferred to turn::finishing; the drain only emits turn_end here.
    expect(rec.state).toBe('finishing');
    expect(rec.turn_end_emitted).toBe(true);
    // 4th arg is the optional model_limit, undefined here (no model_meta on rec).
    expect(emitTurnEnd).toHaveBeenCalledWith('s1', expect.anything(), [], undefined);
    expect(finishSession).not.toHaveBeenCalled();
  });
});
