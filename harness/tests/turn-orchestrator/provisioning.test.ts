import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { TurnOrchestratorConfig } from '../../src/turn-orchestrator/config.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { TurnStepPayloadSchema } from '../../src/turn-orchestrator/schemas.js';
import {
  handleProvisioning,
  parseDirectoryBody,
  register,
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
      mode: null,
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

    vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({
      provider: '',
      model: '',
      mode: null,
      system_prompt: '',
    });
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

describe('register', () => {
  const cfg: TurnOrchestratorConfig = { system_default_skills: [] };

  type Handler = (payload: unknown) => Promise<unknown>;

  function captureHandler(): { iii: ISdk; getHandler: () => Handler; getId: () => string } {
    let handler: Handler | null = null;
    let registeredId = '';
    const iii = {
      registerFunction: (id: string, fn: Handler) => {
        registeredId = id;
        handler = fn;
        return { unregister: () => {} };
      },
      trigger: async () => null,
    } as unknown as ISdk;
    return {
      iii,
      getHandler: () => {
        if (!handler) throw new Error('handler not registered');
        return handler;
      },
      getId: () => registeredId,
    };
  }

  it('registers turn::provisioning, threads cfg into the runner, and returns metadata', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);
    const saveRecord = vi.spyOn(persistence, 'saveRecord').mockResolvedValue();
    const loadRunRequest = vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({
      provider: '',
      model: '',
      mode: null,
      system_prompt: '',
    });
    vi.spyOn(persistence, 'saveFunctionSchemas').mockResolvedValue();
    vi.spyOn(persistence, 'saveRunRequest').mockResolvedValue();

    const { iii, getHandler, getId } = captureHandler();
    register(iii, cfg);
    expect(getId()).toBe('turn::provisioning');

    const result = await getHandler()({ session_id: 's1' });

    // cfg flows through to handleProvisioning (which reads the run request),
    // and the runner threads the pre-mutation snapshot into saveRecord.
    expect(loadRunRequest).toHaveBeenCalledWith(iii, 's1');
    expect(saveRecord).toHaveBeenCalledWith(
      iii,
      rec,
      expect.objectContaining({ state: 'provisioning' }),
    );
    expect(result).toEqual({
      ok: true,
      from_state: 'provisioning',
      to_state: 'assistant_streaming',
    });
  });

  it('rejects payloads missing session_id', async () => {
    const { iii, getHandler } = captureHandler();
    register(iii, cfg);
    await expect(getHandler()({})).rejects.toThrow();
  });
});
