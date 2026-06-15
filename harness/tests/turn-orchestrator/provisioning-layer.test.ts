import { describe, expect, it, vi } from 'vitest';
import type { Model } from '../../src/types/model.js';
import type { ProvisioningPorts } from '../../src/turn-orchestrator/provisioning/ports.js';
import {
  applyProvisioningOutcome,
  processProvisioning,
} from '../../src/turn-orchestrator/provisioning/process.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

const FAKE_MODEL: Model = {
  id: 'gpt-4',
  provider: 'openai',
  api: 'openai-responses',
  display_name: 'GPT-4',
  context_window: 200_000,
  max_output_tokens: 16_000,
};

function stubPorts(overrides: Partial<ProvisioningPorts> = {}): ProvisioningPorts {
  return {
    loadRunRequest: vi.fn(async () => ({
      provider: '',
      model: '',
      mode: null,
      system_prompt: '',
      function_schemas: [],
    })),
    saveRunRequest: vi.fn(async () => {}),
    route: vi.fn(async (provider: string, _model: string) => provider || 'anthropic'),
    resolveModel: vi.fn(async () => null),
    ...overrides,
  };
}

describe('processProvisioning', () => {
  it('builds prompt with mode and attaches agent_trigger schema', async () => {
    const ports = stubPorts({
      loadRunRequest: vi.fn(async () => ({
        provider: 'openai',
        model: 'gpt-4',
        mode: 'agent' as const,
        system_prompt: '',
        function_schemas: [],
      })),
    });
    const rec = { ...newRecord('s1'), state: 'provisioning' as const };

    const outcome = await processProvisioning(ports, rec);

    expect(outcome.kind).toBe('ready');
    expect(outcome.runRequest.system_prompt).toContain('operating in agent mode');
    expect(outcome.runRequest.system_prompt).toContain('You are an iii agent worker');
    expect(outcome.runRequest.function_schemas).toEqual([
      expect.objectContaining({ name: 'agent_trigger' }),
    ]);
  });

  it('preserves a non-empty caller override verbatim', async () => {
    const ports = stubPorts({
      loadRunRequest: vi.fn(async () => ({
        provider: '',
        model: '',
        mode: null,
        system_prompt: 'custom override',
        function_schemas: [],
      })),
    });
    const rec = { ...newRecord('s1'), state: 'provisioning' as const };

    const outcome = await processProvisioning(ports, rec);

    expect(outcome.runRequest.system_prompt).toBe('custom override');
  });

  it('resolves the model once against the ROUTED provider and pins it on the request', async () => {
    const route = vi.fn(async () => 'openai');
    const resolveModel = vi.fn(async () => FAKE_MODEL);
    const ports = stubPorts({
      loadRunRequest: vi.fn(async () => ({
        provider: '',
        model: 'gpt-4',
        mode: null,
        system_prompt: '',
        function_schemas: [],
      })),
      route,
      resolveModel,
    });
    const rec = { ...newRecord('s1'), state: 'provisioning' as const };

    const outcome = await processProvisioning(ports, rec);

    expect(route).toHaveBeenCalledTimes(1);
    expect(route).toHaveBeenCalledWith('', 'gpt-4');
    expect(resolveModel).toHaveBeenCalledTimes(1);
    expect(resolveModel).toHaveBeenCalledWith('openai', 'gpt-4');
    expect(outcome.runRequest.routed_provider).toBe('openai');
    expect(outcome.model_meta).toEqual(FAKE_MODEL);
  });

  it('skips model resolution and serves the default-family prompt when routing fails', async () => {
    const route = vi.fn(async () => null);
    const resolveModel = vi.fn(async () => FAKE_MODEL);
    const ports = stubPorts({
      loadRunRequest: vi.fn(async () => ({
        provider: '',
        model: 'gpt-4',
        mode: null,
        system_prompt: '',
        function_schemas: [],
      })),
      route,
      resolveModel,
    });
    const rec = { ...newRecord('s1'), state: 'provisioning' as const };

    const outcome = await processProvisioning(ports, rec);

    expect(resolveModel).not.toHaveBeenCalled();
    expect(outcome.runRequest.routed_provider).toBe('');
    expect(outcome.model_meta).toBeNull();
  });

  it('does not resolve when there is no model id', async () => {
    const resolveModel = vi.fn(async () => FAKE_MODEL);
    const ports = stubPorts({ resolveModel });
    const rec = { ...newRecord('s1'), state: 'provisioning' as const };

    const outcome = await processProvisioning(ports, rec);

    expect(resolveModel).not.toHaveBeenCalled();
    expect(outcome.model_meta).toBeNull();
  });
});

describe('applyProvisioningOutcome', () => {
  it('saves run request, persists model_meta on the record, and transitions', async () => {
    const saveRunRequest = vi.fn(async () => {});
    const ports = stubPorts({ saveRunRequest });
    const rec = { ...newRecord('s1'), state: 'provisioning' as const };
    const runRequest = {
      provider: 'openai',
      model: 'gpt-4',
      mode: 'agent' as const,
      system_prompt: 'prompt',
      function_schemas: [],
    };

    await applyProvisioningOutcome(ports, rec, {
      kind: 'ready',
      runRequest,
      model_meta: FAKE_MODEL,
    });

    expect(saveRunRequest).toHaveBeenCalledWith('s1', runRequest);
    expect(rec.model_meta).toEqual(FAKE_MODEL);
    expect(rec.state).toBe('assistant_streaming');
  });

  it('leaves model_meta unset when resolution returned null', async () => {
    const ports = stubPorts();
    const rec = { ...newRecord('s1'), state: 'provisioning' as const };
    const runRequest = {
      provider: 'openai',
      model: 'gpt-4',
      mode: 'agent' as const,
      system_prompt: 'prompt',
      function_schemas: [],
    };

    await applyProvisioningOutcome(ports, rec, { kind: 'ready', runRequest, model_meta: null });

    expect(rec.model_meta).toBeUndefined();
    expect(rec.state).toBe('assistant_streaming');
  });
});
