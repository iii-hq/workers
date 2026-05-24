import { describe, it, expect, vi } from 'vitest';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { migrateLegacyRecord } from '../../src/turn-orchestrator/persistence.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

describe('writeRecord', () => {
  it('writes turn_state without emitting turn_state_changed', async () => {
    const calls: string[] = [];
    const iii = {
      trigger: vi.fn(async ({ function_id }: any) => {
        calls.push(function_id);
        return null;
      }),
    } as any;
    await persistence.writeRecord(iii, newRecord('s1'));
    expect(calls).toContain('state::set');
    expect(calls).not.toContain('stream::set'); // no agent::events emit
  });
});

describe('migrateLegacyRecord', () => {
  it('coerces legacy assistant_finished -> function_execute when last_assistant has calls', () => {
    const legacy: any = {
      session_id: 's1', state: 'assistant_finished', turn_count: 1,
      last_assistant: { role: 'assistant', content: [{ type: 'function_call', id: 'c1', function_id: 'x::y', arguments: {} }] },
      pending_function_calls: [], function_results: [], turn_end_emitted: false, started_at_ms: 1, updated_at_ms: 1,
    };
    expect(migrateLegacyRecord(legacy).state).toBe('function_execute');
  });
  it('coerces legacy assistant_finished -> steering_check when no calls', () => {
    const legacy: any = {
      session_id: 's1', state: 'assistant_finished', turn_count: 1,
      last_assistant: { role: 'assistant', content: [{ type: 'text', text: 'hi' }] },
      pending_function_calls: [], function_results: [], turn_end_emitted: false, started_at_ms: 1, updated_at_ms: 1,
    };
    expect(migrateLegacyRecord(legacy).state).toBe('steering_check');
  });
  it('leaves current records untouched', () => {
    const cur: any = { session_id: 's1', state: 'function_execute', turn_count: 1, pending_function_calls: [], function_results: [], turn_end_emitted: false, started_at_ms: 1, updated_at_ms: 1 };
    expect(migrateLegacyRecord(cur).state).toBe('function_execute');
  });
});
