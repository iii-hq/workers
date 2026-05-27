import { afterEach, describe, expect, it, vi } from 'vitest';
import type { TurnOrchestratorConfig } from '../../src/turn-orchestrator/config.js';
import { fakeIii } from './_helpers/fakeIii.js';
import { defaultRunRequest, installMockTurnStore } from './_helpers/mockTurnStore.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { TurnStepPayloadSchema } from '../../src/turn-orchestrator/schemas.js';
import { parseDirectoryBody } from '../../src/turn-orchestrator/provisioning/ports.js';
import { handleProvisioning, register } from '../../src/turn-orchestrator/provisioning/process.js';

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
      responder: {
        'directory::skills::index': { body: 'INDEX' },
        'directory::skills::get': { body: 'SKILL' },
      },
    });
    const cfg = { system_default_skills: ['iii://iii-directory/index'] };

    const store = installMockTurnStore({
      loadRunRequest: vi.fn(async () => ({
        ...defaultRunRequest,
        mode: 'agent',
      })),
    });
    const saveRunRequest = store.saveRunRequest;

    await handleProvisioning(iii, cfg, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(saveRunRequest).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({
        provider: 'openai',
        model: 'gpt-4',
        system_prompt: expect.stringContaining('operating in agent mode'),
        function_schemas: [expect.objectContaining({ name: 'agent_trigger' })],
      }),
    );
    expect(calls.some((c) => c.function_id === 'directory::skills::index')).toBe(true);
    expect(calls.some((c) => c.function_id === 'directory::skills::get')).toBe(true);
  });

  it('preserves a non-empty caller override verbatim', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const { iii } = fakeIii();
    const cfg = { system_default_skills: [] as string[] };

    const store = installMockTurnStore({
      loadRunRequest: vi.fn(async () => ({
        ...defaultRunRequest,
        mode: null,
        system_prompt: 'custom override',
      })),
    });
    const saveRunRequest = store.saveRunRequest;

    await handleProvisioning(iii, cfg, rec);

    expect(saveRunRequest).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ system_prompt: 'custom override' }),
    );
  });

  it('continues when directory fetches fail', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const { iii } = fakeIii();
    const cfg = { system_default_skills: ['iii://missing'] };

    const store = installMockTurnStore({
      loadRunRequest: vi.fn(async () => ({
        ...defaultRunRequest,
        provider: '',
        model: '',
        mode: null,
      })),
    });
    const saveRunRequest = store.saveRunRequest;

    await handleProvisioning(iii, cfg, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(saveRunRequest).toHaveBeenCalledWith(
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
    const store = installMockTurnStore({
      loadRecord: vi.fn(async () => rec),
      loadRunRequest: vi.fn(async () => ({
        ...defaultRunRequest,
        provider: '',
        model: '',
        mode: null,
      })),
    });
    const saveRecord = store.saveRecord;
    const loadRunRequest = store.loadRunRequest;

    const { iii, getHandler, getId } = captureHandler();
    register(iii, cfg);
    expect(getId()).toBe('turn::provisioning');

    const result = await getHandler()({ session_id: 's1' });

    // cfg flows through to handleProvisioning (which reads the run request),
    // and the runner threads the pre-mutation snapshot into saveRecord.
    expect(loadRunRequest).toHaveBeenCalledWith('s1');
    expect(saveRecord).toHaveBeenCalledWith(
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
