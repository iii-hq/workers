import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { _resetSeqForTests, emit } from '../../src/turn-orchestrator/events.js';
import type { AgentEvent } from '../../src/types/agent-event.js';

function buildSdk() {
  const calls: Array<{ function_id: string; payload: Record<string, unknown> }> = [];
  const trigger = vi.fn(async (req: { function_id: string; payload?: unknown }) => {
    calls.push({
      function_id: req.function_id,
      payload: (req.payload ?? {}) as Record<string, unknown>,
    });
    return {};
  });
  return { iii: { trigger } as unknown as ISdk, calls };
}

const SID = 'sess-1';
const UPDATE = (event = 'message_update') => ({ type: event }) as unknown as AgentEvent;
const streamSets = (calls: Array<{ function_id: string; payload: Record<string, unknown> }>) =>
  calls.filter((c) => c.function_id === 'stream::set');
const itemIds = (calls: Array<{ function_id: string; payload: Record<string, unknown> }>) =>
  streamSets(calls).map((c) => c.payload.item_id as string);

beforeEach(() => {
  _resetSeqForTests();
});

describe('emit (agent event producer)', () => {
  it('writes a non-turn_end event only to agent::events', async () => {
    const { iii, calls } = buildSdk();

    await emit(iii, SID, UPDATE());

    expect(streamSets(calls).map((c) => c.payload.stream_name)).toEqual(['agent::events']);
  });

  it('mirrors a turn_end event onto the dedicated agent::turn_end stream', async () => {
    const { iii, calls } = buildSdk();
    const event = {
      type: 'turn_end',
      message: { role: 'assistant' },
      function_results: [],
    } as unknown as AgentEvent;

    await emit(iii, SID, event);

    const sets = streamSets(calls);
    const streams = sets.map((c) => c.payload.stream_name);
    expect(streams).toContain('agent::events');
    expect(streams).toContain('agent::turn_end');

    const mirror = sets.find((c) => c.payload.stream_name === 'agent::turn_end');
    expect(mirror?.payload.group_id).toBe(SID);
    expect(mirror?.payload.data).toEqual(event);
    // Same logical event → identical item_id on both streams (single seq per emit).
    expect(new Set(sets.map((c) => c.payload.item_id)).size).toBe(1);
  });
});

describe('emit (in-process sequence, no persisted counter)', () => {
  it('does NOT issue a state::update — the seq is process-local', async () => {
    const { iii, calls } = buildSdk();

    await emit(iii, SID, UPDATE());

    expect(calls.filter((c) => c.function_id === 'state::update')).toHaveLength(0);
  });

  it('produces a monotonic per-session seq starting at 0', async () => {
    const { iii, calls } = buildSdk();

    await emit(iii, SID, UPDATE());
    await emit(iii, SID, UPDATE());

    const ids = itemIds(calls);
    expect(ids[0].endsWith('-00000000')).toBe(true);
    expect(ids[1].endsWith('-00000001')).toBe(true);
  });

  it('counts each session independently (both start at 0)', async () => {
    const { iii, calls } = buildSdk();

    await emit(iii, 'sess-a', UPDATE());
    await emit(iii, 'sess-b', UPDATE());

    const a = itemIds(calls).find((id) => id.startsWith('sess-a-'));
    const b = itemIds(calls).find((id) => id.startsWith('sess-b-'));
    expect(a?.endsWith('-00000000')).toBe(true);
    expect(b?.endsWith('-00000000')).toBe(true);
  });

  it('formats item_id as <session>-<process_epoch>-<8 digit seq> with a stable epoch', async () => {
    const { iii, calls } = buildSdk();

    await emit(iii, SID, UPDATE());
    await emit(iii, SID, UPDATE());

    const ids = itemIds(calls);
    expect(ids[0]).toMatch(new RegExp(`^${SID}-.+-\\d{8}$`));
    // epoch = substring between the `<sid>-` prefix and the trailing `-<8 digits>`
    const epochOf = (id: string) => id.slice(`${SID}-`.length, id.length - '-00000000'.length);
    expect(epochOf(ids[0]).length).toBeGreaterThan(0);
    expect(epochOf(ids[0])).toBe(epochOf(ids[1])); // stable within a process
  });
});
