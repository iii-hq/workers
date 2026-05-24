import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { execute } from '../../src/turn-orchestrator/get-state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import { GetStatePayloadSchema } from '../../src/turn-orchestrator/schemas.js';

describe('GetStatePayloadSchema', () => {
  it('accepts the flat shape the real backend sends', () => {
    expect(GetStatePayloadSchema.parse({ session_id: 'sess-abc' })).toEqual({
      session_id: 'sess-abc',
    });
  });

  it('strips extra keys (engine may add metadata later)', () => {
    expect(GetStatePayloadSchema.parse({ session_id: 's1', trace_id: 't1' })).toEqual({
      session_id: 's1',
    });
  });

  it('rejects publish envelope shapes — no in-repo caller wraps get_state', () => {
    expect(() =>
      GetStatePayloadSchema.parse({
        topic: 'turn::step_requested',
        data: { session_id: 's1' },
      }),
    ).toThrow();
  });

  it('rejects nested payload wrappers (no in-repo caller uses them)', () => {
    expect(() => GetStatePayloadSchema.parse({ data: { session_id: 's1' } })).toThrow();
    expect(() => GetStatePayloadSchema.parse({ payload: { session_id: 's1' } })).toThrow();
  });

  it('rejects missing, empty, or non-string session_id', () => {
    expect(() => GetStatePayloadSchema.parse({})).toThrow();
    expect(() => GetStatePayloadSchema.parse({ session_id: '' })).toThrow();
    expect(() => GetStatePayloadSchema.parse({ session_id: 42 })).toThrow();
    expect(() => GetStatePayloadSchema.parse({ session_id: null })).toThrow();
    expect(() => GetStatePayloadSchema.parse(null)).toThrow();
    expect(() => GetStatePayloadSchema.parse(undefined)).toThrow();
  });
});

describe('turn::get_state execute', () => {
  it('returns the turn_state record for a known session via persistence.loadRecord', async () => {
    const rec = newRecord('sess-abc');
    rec.state = 'function_awaiting_approval';
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        if (
          req.function_id === 'state::get' &&
          (req.payload as Record<string, unknown>).key === 'session/sess-abc/turn_state'
        ) {
          return rec;
        }
        return null;
      }),
    } as unknown as ISdk;

    const out = await execute(iii, { session_id: 'sess-abc' });
    expect(out).toEqual(rec);
  });

  it('returns null when no record exists for the session', async () => {
    const iii = {
      trigger: vi.fn(async () => null),
    } as unknown as ISdk;
    const out = await execute(iii, { session_id: 'unknown' });
    expect(out).toBeNull();
  });
});
