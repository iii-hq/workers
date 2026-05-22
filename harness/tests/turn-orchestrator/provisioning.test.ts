import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { TurnOrchestratorConfig } from '../../src/turn-orchestrator/config.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { TurnStepPayloadSchema } from '../../src/turn-orchestrator/turn-step-payload.js';
import {
  execute,
  handleProvisioning,
  parseDirectoryBody,
  parseRunRequest,
} from '../../src/turn-orchestrator/states/provisioning.js';

type TriggerCall = { function_id: string; payload: unknown; timeoutMs?: number };

function fakeIii(responses: Record<string, unknown> = {}): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: {
      function_id: string;
      payload: T;
      timeoutMs?: number;
    }): Promise<R> => {
      calls.push({
        function_id: req.function_id,
        payload: req.payload,
        timeoutMs: req.timeoutMs,
      });
      return (responses[req.function_id] ?? null) as R;
    },
  } as unknown as ISdk;
  return { iii, calls };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('parseRunRequest', () => {
  it('maps persisted run::start fields with defaults for missing keys', () => {
    expect(parseRunRequest({})).toEqual({
      provider: '',
      model: '',
      mode: null,
      system_prompt: '',
      image: 'python',
      idle_timeout_secs: 300,
    });
  });

  it('rejects invalid mode values', () => {
    expect(parseRunRequest({ mode: 'invalid' }).mode).toBeNull();
    expect(parseRunRequest({ mode: 'plan' }).mode).toBe('plan');
  });
});

describe('parseDirectoryBody', () => {
  it('accepts bare string and wrapped body responses', () => {
    expect(parseDirectoryBody('raw')).toBe('raw');
    expect(parseDirectoryBody({ body: 'wrapped' })).toBe('wrapped');
  });

  it('rejects empty wrapped body and non-string shapes', () => {
    expect(parseDirectoryBody({ body: '' })).toBe('');
    expect(parseDirectoryBody({ body: 1 })).toBeNull();
    expect(parseDirectoryBody(null)).toBeNull();
  });
});

describe('handleProvisioning', () => {
  it('materializes schemas, persists built prompt, and advances to assistant_streaming', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const { iii, calls } = fakeIii({
      'directory::skills::index': { body: 'INDEX' },
      'directory::skills::get': { body: 'SKILL' },
    });
    const cfg = { system_default_skills: ['iii://iii-directory/index'] };

    vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({
      provider: 'openai',
      model: 'gpt-4',
      mode: 'agent',
      system_prompt: '',
      image: 'python',
      idle_timeout_secs: 300,
    });
    const saveSchemas = vi.spyOn(persistence, 'saveFunctionSchemas').mockResolvedValue();
    const saveRunRequest = vi.spyOn(persistence, 'saveRunRequest').mockResolvedValue();

    await handleProvisioning(iii, cfg, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(saveSchemas).toHaveBeenCalledWith(iii, 's1', [
      expect.objectContaining({ name: 'agent_trigger' }),
    ]);
    expect(saveRunRequest).toHaveBeenCalledWith(
      iii,
      's1',
      expect.objectContaining({
        provider: 'openai',
        model: 'gpt-4',
        system_prompt: expect.stringContaining('operating in agent mode'),
      }),
    );
    expect(calls.some((c) => c.function_id === 'directory::skills::index')).toBe(true);
    expect(calls.some((c) => c.function_id === 'directory::skills::get')).toBe(true);
  });

  it('preserves a non-empty caller override verbatim', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const { iii } = fakeIii();
    const cfg = { system_default_skills: [] as string[] };

    vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({
      provider: 'openai',
      model: 'gpt-4',
      system_prompt: 'custom override',
    });
    vi.spyOn(persistence, 'saveFunctionSchemas').mockResolvedValue();
    const saveRunRequest = vi.spyOn(persistence, 'saveRunRequest').mockResolvedValue();

    await handleProvisioning(iii, cfg, rec);

    expect(saveRunRequest).toHaveBeenCalledWith(
      iii,
      's1',
      expect.objectContaining({ system_prompt: 'custom override' }),
    );
  });

  it('continues when directory fetches fail', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const { iii } = fakeIii();
    const cfg = { system_default_skills: ['iii://missing'] };

    vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({});
    vi.spyOn(persistence, 'saveFunctionSchemas').mockResolvedValue();
    const saveRunRequest = vi.spyOn(persistence, 'saveRunRequest').mockResolvedValue();

    await handleProvisioning(iii, cfg, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(saveRunRequest).toHaveBeenCalledWith(
      iii,
      's1',
      expect.objectContaining({
        system_prompt: expect.stringContaining('You are an iii agent worker'),
      }),
    );
  });
});

describe('TurnStepPayloadSchema', () => {
  it('accepts the flat shape every in-repo caller uses', () => {
    expect(TurnStepPayloadSchema.parse({ session_id: 's1' })).toEqual({ session_id: 's1' });
  });
});

describe('execute', () => {
  const cfg: TurnOrchestratorConfig = { system_default_skills: [] };

  it('throws when the session record is missing', async () => {
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(null);

    await expect(execute({} as ISdk, cfg, { session_id: 'missing' })).rejects.toThrow(
      'turn::provisioning invariant: missing session missing',
    );
  });

  it('returns stale skip when persisted state drifted', async () => {
    const rec = { ...newRecord('s1'), state: 'assistant_streaming' as const };
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);
    const saveRecord = vi.spyOn(persistence, 'saveRecord').mockResolvedValue();

    const result = await execute({} as ISdk, cfg, { session_id: 's1' });

    expect(result).toEqual({ ok: true, skipped: true, reason: 'stale' });
    expect(saveRecord).not.toHaveBeenCalled();
  });

  it('runs handleProvisioning, saves the record, and returns transition metadata', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const { iii } = fakeIii();
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);
    const saveRecord = vi.spyOn(persistence, 'saveRecord').mockResolvedValue();
    vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({});
    vi.spyOn(persistence, 'saveFunctionSchemas').mockResolvedValue();
    vi.spyOn(persistence, 'saveRunRequest').mockResolvedValue();

    const result = await execute(iii, cfg, { session_id: 's1' });

    expect(saveRecord).toHaveBeenCalledWith(iii, rec);
    expect(result).toEqual({
      ok: true,
      from_state: 'provisioning',
      to_state: 'assistant_streaming',
    });
  });

  it('wraps handler failures as transition errors', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);
    vi.spyOn(persistence, 'saveRecord').mockResolvedValue();
    vi.spyOn(persistence, 'loadRunRequest').mockRejectedValue(new Error('boom'));

    await expect(execute({} as ISdk, cfg, { session_id: 's1' })).rejects.toThrow(
      'transition from provisioning failed: Error: boom',
    );
  });
});
