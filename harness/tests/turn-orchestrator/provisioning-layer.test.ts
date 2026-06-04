import { describe, expect, it, vi } from 'vitest';
import { applyProvisioningOutcome } from '../../src/turn-orchestrator/provisioning/process.js';
import type { ProvisioningPorts } from '../../src/turn-orchestrator/provisioning/ports.js';
import { processProvisioning } from '../../src/turn-orchestrator/provisioning/process.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

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
});

describe('applyProvisioningOutcome', () => {
  it('saves run request and transitions to assistant_streaming', async () => {
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

    await applyProvisioningOutcome(ports, rec, { kind: 'ready', runRequest });

    expect(saveRunRequest).toHaveBeenCalledWith('s1', runRequest);
    expect(rec.state).toBe('assistant_streaming');
  });
});
