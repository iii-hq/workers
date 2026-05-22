import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { TurnOrchestratorConfig } from '../../src/turn-orchestrator/config.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { newRecord, turnStateKey } from '../../src/turn-orchestrator/state.js';
import * as transitions from '../../src/turn-orchestrator/transitions.js';
import { execute, shouldStep, StepPayloadSchema } from '../../src/turn-orchestrator/subscriber.js';

const cfg: TurnOrchestratorConfig = { system_default_skills: [] };

afterEach(() => {
  vi.restoreAllMocks();
});

describe('StepPayloadSchema', () => {
  it('accepts the flat shape every in-repo caller uses', () => {
    expect(StepPayloadSchema.parse({ session_id: 'sess-abc' })).toEqual({
      session_id: 'sess-abc',
    });
  });

  it('strips extra keys (engine may add metadata later)', () => {
    expect(StepPayloadSchema.parse({ session_id: 's1', trace_id: 't1' })).toEqual({
      session_id: 's1',
    });
  });

  it('rejects publish envelope shapes — durable subscriber receives data only', () => {
    expect(() =>
      StepPayloadSchema.parse({
        topic: 'turn::step_requested',
        data: { session_id: 's1' },
      }),
    ).toThrow();
  });

  it('rejects nested payload wrappers (no in-repo caller uses them)', () => {
    expect(() => StepPayloadSchema.parse({ data: { session_id: 's1' } })).toThrow();
    expect(() => StepPayloadSchema.parse({ payload: { session_id: 's1' } })).toThrow();
  });

  it('rejects missing, empty, or non-string session_id', () => {
    expect(() => StepPayloadSchema.parse({})).toThrow();
    expect(() => StepPayloadSchema.parse({ session_id: '' })).toThrow();
    expect(() => StepPayloadSchema.parse({ session_id: 42 })).toThrow();
    expect(() => StepPayloadSchema.parse({ session_id: null })).toThrow();
    expect(() => StepPayloadSchema.parse(null)).toThrow();
    expect(() => StepPayloadSchema.parse(undefined)).toThrow();
  });
});

describe('shouldStep', () => {
  it('returns false when the record does not exist', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(null);

    await expect(shouldStep(iii, { session_id: 'missing' })).resolves.toBe(false);
  });

  it('returns false when the record is stopped', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'stopped';
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);

    await expect(shouldStep(iii, { session_id: 's1' })).resolves.toBe(false);
  });

  it('returns true for a known non-terminal session', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'provisioning';
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);

    await expect(shouldStep(iii, { session_id: 's1' })).resolves.toBe(true);
  });

  it('rejects malformed payloads', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const loadSpy = vi.spyOn(persistence, 'loadRecord');

    await expect(shouldStep(iii, {})).resolves.toBe(false);
    expect(loadSpy).not.toHaveBeenCalled();
  });
});

function mockTurnStateGet(iii: ISdk, rec: ReturnType<typeof newRecord> | null): void {
  vi.mocked(iii.trigger).mockImplementation(async (req) => {
    if (
      req.function_id === 'state::get' &&
      req.payload &&
      typeof req.payload === 'object' &&
      (req.payload as { key?: string }).key === turnStateKey(rec?.session_id ?? 'missing')
    ) {
      return rec;
    }
    throw new Error(`unexpected trigger ${req.function_id}`);
  });
}

describe('execute', () => {
  it('throws when the record does not exist', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    mockTurnStateGet(iii, null);

    await expect(execute(iii, cfg, { session_id: 'missing' })).rejects.toThrow(
      'turn::step invariant: missing session missing',
    );
  });

  it('steps, persists, and returns from_state/to_state on success', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'provisioning';
    mockTurnStateGet(iii, rec);
    vi.spyOn(transitions, 'step').mockImplementation(async (_iii, _cfg, r) => {
      r.state = 'awaiting_assistant';
    });
    const saveSpy = vi.spyOn(persistence, 'saveRecord').mockResolvedValue(undefined);

    await expect(execute(iii, cfg, { session_id: 's1' })).resolves.toEqual({
      ok: true,
      from_state: 'provisioning',
      to_state: 'awaiting_assistant',
    });
    expect(saveSpy).toHaveBeenCalledWith(iii, rec);
  });

  it('throws with from_state when transition fails', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_execute';
    mockTurnStateGet(iii, rec);
    vi.spyOn(transitions, 'step').mockRejectedValue(new Error('sandbox gone'));

    await expect(execute(iii, cfg, { session_id: 's1' })).rejects.toThrow(
      'transition from function_execute failed: Error: sandbox gone',
    );
  });
});
