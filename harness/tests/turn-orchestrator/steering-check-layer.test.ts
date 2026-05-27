import { describe, expect, it, vi } from 'vitest';
import type { AgentMessage } from '../../src/types/agent-message.js';
import { applySteeringCheckOutcome, processSteeringCheck } from '../../src/turn-orchestrator/steering-check/run.js';
import { parseDrainItems } from '../../src/turn-orchestrator/steering-check/ports.js';
import type { SteeringCheckPorts } from '../../src/turn-orchestrator/steering-check/ports.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

function userMessage(text: string): AgentMessage {
  return { role: 'user', content: [{ type: 'text', text }] };
}

function stubPorts(overrides: Partial<SteeringCheckPorts> = {}): SteeringCheckPorts {
  return {
    drainInbox: vi.fn(async () => []),
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

describe('parseDrainItems', () => {
  it('returns items array when present', () => {
    const items = [userMessage('hello')];
    expect(parseDrainItems({ items })).toEqual(items);
  });

  it('returns empty array for invalid shapes', () => {
    expect(parseDrainItems(null)).toEqual([]);
    expect(parseDrainItems({})).toEqual([]);
    expect(parseDrainItems({ items: 'bad' })).toEqual([]);
  });
});

describe('processSteeringCheck', () => {
  it('returns resume_with_inbox for steering messages', async () => {
    const steeringItems = [userMessage('steer')];
    const ports = stubPorts({
      drainInbox: vi.fn(async (name) => (name === 'steering' ? steeringItems : [])),
    });
    const rec = { ...newRecord('s1'), state: 'steering_check' as const };

    const outcome = await processSteeringCheck(ports, rec);

    expect(outcome).toEqual({ kind: 'resume_with_inbox', inbox: steeringItems });
    expect(ports.drainInbox).toHaveBeenCalledTimes(1);
  });

  it('drains followup only when steering is empty', async () => {
    const followupItems = [userMessage('follow')];
    const drainInbox = vi.fn(async (name: 'steering' | 'followup') =>
      name === 'followup' ? followupItems : [],
    );
    const ports = stubPorts({ drainInbox });
    const rec = { ...newRecord('s1'), state: 'steering_check' as const };

    const outcome = await processSteeringCheck(ports, rec);

    expect(outcome).toEqual({ kind: 'resume_with_inbox', inbox: followupItems });
    expect(drainInbox).toHaveBeenCalledWith('steering', 's1');
    expect(drainInbox).toHaveBeenCalledWith('followup', 's1');
  });

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

  it('returns end_turn when no steering, followup, or function results', async () => {
    const ports = stubPorts();
    const rec = { ...newRecord('s1'), state: 'steering_check' as const };

    const outcome = await processSteeringCheck(ports, rec);

    expect(outcome).toEqual({ kind: 'end_turn' });
  });
});

describe('applySteeringCheckOutcome', () => {
  it('resume_with_inbox: emits turn_end, saves messages, clears function_results', async () => {
    const inbox = [userMessage('new')];
    const emitTurnEnd = vi.fn(async () => {});
    const appendMessages = vi.fn(async () => {});
    const ports = stubPorts({
      emitTurnEnd,
      appendMessages,
    });
    const rec = {
      ...newRecord('s1'),
      state: 'steering_check' as const,
      function_results: [{ role: 'function_result', content: [] }] as never,
    };

    await applySteeringCheckOutcome(ports, rec, { kind: 'resume_with_inbox', inbox });

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
    expect(rec.turn_end_emitted).toBe(true);
    expect(emitTurnEnd).toHaveBeenCalledWith('s1', expect.anything(), []);
    expect(appendMessages).toHaveBeenCalledWith('s1', inbox);
  });

  it('continue_after_function: transitions without loading messages', async () => {
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

  it('end_turn: emits turn_end and finishes session', async () => {
    const emitTurnEnd = vi.fn(async () => {});
    const finishSession = vi.fn(async (rec) => {
      rec.state = 'stopped';
    });
    const ports = stubPorts({ emitTurnEnd, finishSession });
    const rec = { ...newRecord('s1'), state: 'steering_check' as const };

    await applySteeringCheckOutcome(ports, rec, { kind: 'end_turn' });

    expect(rec.state).toBe('stopped');
    expect(rec.turn_end_emitted).toBe(true);
    expect(emitTurnEnd).toHaveBeenCalledWith('s1', expect.anything(), []);
    expect(finishSession).toHaveBeenCalled();
  });
});
