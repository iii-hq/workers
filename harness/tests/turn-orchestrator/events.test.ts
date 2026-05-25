import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { emit } from '../../src/turn-orchestrator/events.js';
import type { AgentEvent } from '../../src/types/agent-event.js';

function buildSdk() {
  const calls: Array<{ function_id: string; payload: Record<string, unknown> }> = [];
  const trigger = vi.fn(async (req: { function_id: string; payload?: unknown }) => {
    calls.push({
      function_id: req.function_id,
      payload: (req.payload ?? {}) as Record<string, unknown>,
    });
    if (req.function_id === 'state::update') return { old_value: 0 };
    return {};
  });
  return { iii: { trigger } as unknown as ISdk, calls };
}

const SID = 'sess-1';

describe('emit (agent event producer)', () => {
  it('writes a non-turn_end event only to agent::events', async () => {
    const { iii, calls } = buildSdk();
    const event = { type: 'message_update' } as unknown as AgentEvent;

    await emit(iii, SID, event);

    const sets = calls.filter((c) => c.function_id === 'stream::set');
    expect(sets.map((c) => c.payload.stream_name)).toEqual(['agent::events']);
  });

  it('mirrors a turn_end event onto the dedicated agent::turn_end stream', async () => {
    const { iii, calls } = buildSdk();
    const event = {
      type: 'turn_end',
      message: { role: 'assistant' },
      function_results: [],
    } as unknown as AgentEvent;

    await emit(iii, SID, event);

    const sets = calls.filter((c) => c.function_id === 'stream::set');
    const streams = sets.map((c) => c.payload.stream_name);
    expect(streams).toContain('agent::events');
    expect(streams).toContain('agent::turn_end');

    const mirror = sets.find((c) => c.payload.stream_name === 'agent::turn_end');
    expect(mirror?.payload.group_id).toBe(SID);
    expect(mirror?.payload.data).toEqual(event);
  });
});
