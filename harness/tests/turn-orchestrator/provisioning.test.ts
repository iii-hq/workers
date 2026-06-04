import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { defaultRunRequest, installMockTurnStore } from './_helpers/mockTurnStore.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { TurnStepPayloadSchema } from '../../src/turn-orchestrator/schemas.js';
import { handleProvisioning, register } from '../../src/turn-orchestrator/provisioning/process.js';

function fakeIii(): ISdk {
  return {
    trigger: async () => null,
  } as unknown as ISdk;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('handleProvisioning', () => {
  it('materializes schemas, persists built prompt, and advances to assistant_streaming', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const iii = fakeIii();

    const store = installMockTurnStore({
      loadRunRequest: vi.fn(async () => ({
        ...defaultRunRequest,
        mode: 'agent' as const,
      })),
    });
    const saveRunRequest = store.saveRunRequest;

    await handleProvisioning(iii, rec);

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
  });

  it('preserves a non-empty caller override verbatim', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const iii = fakeIii();

    const store = installMockTurnStore({
      loadRunRequest: vi.fn(async () => ({
        ...defaultRunRequest,
        mode: null,
        system_prompt: 'custom override',
      })),
    });
    const saveRunRequest = store.saveRunRequest;

    await handleProvisioning(iii, rec);

    expect(saveRunRequest).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ system_prompt: 'custom override' }),
    );
  });

  it('builds the canonical preamble when no override or mode is set', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const iii = fakeIii();

    const store = installMockTurnStore({
      loadRunRequest: vi.fn(async () => ({
        ...defaultRunRequest,
        provider: '',
        model: '',
        mode: null,
      })),
    });
    const saveRunRequest = store.saveRunRequest;

    await handleProvisioning(iii, rec);

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

  it('registers turn::provisioning, runs the transition, and returns metadata', async () => {
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
    register(iii);
    expect(getId()).toBe('turn::provisioning');

    const result = await getHandler()({ session_id: 's1' });

    // The runner reads the run request and threads the pre-mutation snapshot
    // into saveRecord.
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
    register(iii);
    await expect(getHandler()({})).rejects.toThrow();
  });
});
