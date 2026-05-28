import { describe, expect, it } from 'vitest';
import { TurnStateInvariantError } from '../../src/turn-orchestrator/errors.js';
import {
  parseAssistantStreamingRecord,
  parseFunctionBatchRecord,
  parseSteeringCheckRecord,
} from '../../src/turn-orchestrator/schemas.js';
import {
  type TurnStateRecord,
  newRecord,
  transitionTo,
} from '../../src/turn-orchestrator/state.js';
import { enterFunctionExecute } from '../../src/turn-orchestrator/function-execute/run.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';

describe('TurnStateRecord', () => {
  it('starts in provisioning with no work and the given max_turns', () => {
    const r = newRecord('s1', 32);
    expect(r.state).toBe('provisioning');
    expect(r.session_id).toBe('s1');
    expect(r.max_turns).toBe(32);
    expect(r.work).toBeUndefined();
  });

  it('transitionTo stopped marks terminal', () => {
    const r = newRecord('s1');
    transitionTo(r, 'stopped');
    expect(r.state).toBe('stopped');
  });

  it('awaiting_approval defaults to undefined on fresh records', () => {
    const rec: TurnStateRecord = newRecord('s1');
    expect(rec.awaiting_approval).toBeUndefined();
  });
});

describe('parseFunctionBatchRecord', () => {
  const asst: AssistantMessage = {
    role: 'assistant',
    content: [],
    stop_reason: 'function_call',
    error_message: null,
    error_kind: null,
    usage: null,
    model: 'm',
    provider: 'p',
    timestamp: 1,
  };

  it('returns a validated record when function-batch fields are present', () => {
    const rec = newRecord('s1');
    enterFunctionExecute(rec, asst);
    rec.state = 'function_execute';
    const batch = parseFunctionBatchRecord(rec);
    expect(batch.work).toBeDefined();
    expect(batch.awaiting_approval).toEqual([]);
  });

  it('throws TurnStateInvariantError when last_assistant is missing', () => {
    const rec = newRecord('s1');
    rec.state = 'function_execute';
    rec.work = { prepared: [], executed: {} };
    rec.awaiting_approval = [];
    expect(() => parseFunctionBatchRecord(rec)).toThrow(TurnStateInvariantError);
  });
});

describe('parseAssistantStreamingRecord', () => {
  it('returns a validated record for assistant_streaming', () => {
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';
    const streaming = parseAssistantStreamingRecord(rec);
    expect(streaming.state).toBe('assistant_streaming');
    expect(streaming.function_results).toEqual([]);
  });

  it('throws TurnStateInvariantError when session_id is missing', () => {
    const rec = { state: 'assistant_streaming' } as TurnStateRecord;
    expect(() => parseAssistantStreamingRecord(rec)).toThrow(TurnStateInvariantError);
  });

  it('throws TurnStateInvariantError when state is wrong', () => {
    const rec = newRecord('s1');
    rec.state = 'provisioning';
    expect(() => parseAssistantStreamingRecord(rec)).toThrow(TurnStateInvariantError);
  });
});

describe('parseSteeringCheckRecord', () => {
  it('returns a validated record for steering_check', () => {
    const rec = newRecord('s1');
    rec.state = 'steering_check';
    const steering = parseSteeringCheckRecord(rec);
    expect(steering.state).toBe('steering_check');
    expect(steering.function_results).toEqual([]);
  });

  it('throws TurnStateInvariantError when session_id is missing', () => {
    const rec = { state: 'steering_check' } as TurnStateRecord;
    expect(() => parseSteeringCheckRecord(rec)).toThrow(TurnStateInvariantError);
  });
});
