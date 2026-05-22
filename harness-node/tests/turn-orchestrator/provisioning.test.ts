import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import {
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
  it('materializes schemas, persists built prompt, and advances to awaiting_assistant', async () => {
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

    expect(rec.state).toBe('awaiting_assistant');
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

    expect(rec.state).toBe('awaiting_assistant');
    expect(saveRunRequest).toHaveBeenCalledWith(
      iii,
      's1',
      expect.objectContaining({
        system_prompt: expect.stringContaining('You are an iii agent worker'),
      }),
    );
  });
});
