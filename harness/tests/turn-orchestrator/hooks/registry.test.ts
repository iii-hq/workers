import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ISdk, TriggerConfig, TriggerHandler } from '../../../src/runtime/iii.js';
import {
  addPreTriggerBindingForTests,
  registerPreTriggerType,
  resetPreTriggerBindingsForTests,
  snapshotBindings,
} from '../../../src/turn-orchestrator/hooks/registry.js';
import { PRE_TRIGGER_TYPE } from '../../../src/turn-orchestrator/hooks/types.js';

type CapturedHandler = TriggerHandler<unknown>;

function captureHandler(): { iii: ISdk; handler: () => CapturedHandler } {
  let handler: CapturedHandler | undefined;
  const iii = {
    registerTriggerType: vi.fn((input: { id: string }, h: CapturedHandler) => {
      expect(input.id).toBe(PRE_TRIGGER_TYPE);
      handler = h;
      return {};
    }),
  } as unknown as ISdk;
  return {
    iii,
    handler: () => {
      if (!handler) throw new Error('trigger type not registered');
      return handler;
    },
  };
}

function bindingConfig(id: string, function_id: string, config: unknown): TriggerConfig<unknown> {
  return { id, function_id, config };
}

beforeEach(() => {
  resetPreTriggerBindingsForTests();
});

describe('pre_trigger trigger type registry', () => {
  it('registers and unregisters bindings through the engine handler', async () => {
    const { iii, handler } = captureHandler();
    registerPreTriggerType(iii);

    await handler().registerTrigger(bindingConfig('b1', 'approval::gate', { functions: ['*'] }));
    expect(snapshotBindings('shell::run')).toHaveLength(1);

    await handler().unregisterTrigger(bindingConfig('b1', 'approval::gate', {}));
    expect(snapshotBindings('shell::run')).toHaveLength(0);
  });

  it('rejects a malformed binding config at registration (strict schema)', async () => {
    const { iii, handler } = captureHandler();
    registerPreTriggerType(iii);

    await expect(
      handler().registerTrigger(bindingConfig('b1', 'approval::gate', { functoins: ['*'] })),
    ).rejects.toThrow();
    expect(snapshotBindings('shell::run')).toHaveLength(0);
  });

  it('treats a null config as match-all with defaults', async () => {
    const { iii, handler } = captureHandler();
    registerPreTriggerType(iii);

    await handler().registerTrigger(bindingConfig('b1', 'approval::gate', null));
    const [binding] = snapshotBindings('anything::at_all');
    expect(binding).toBeDefined();
    expect(binding?.config.timeout_ms).toBe(5_000);
    expect(binding?.config.on_error).toBe('fail_closed');
  });

  it('narrows by functions globs', () => {
    addPreTriggerBindingForTests({
      id: 'b1',
      function_id: 'approval::gate',
      config: {
        functions: ['shell::*', 'harness::spawn'],
        timeout_ms: 5_000,
        on_error: 'fail_closed',
      },
    });

    expect(snapshotBindings('shell::run')).toHaveLength(1);
    expect(snapshotBindings('shell::fs::read')).toHaveLength(1);
    expect(snapshotBindings('harness::spawn')).toHaveLength(1);
    expect(snapshotBindings('web::fetch')).toHaveLength(0);
  });

  it('orders the chain by priority, ties by function_id', () => {
    addPreTriggerBindingForTests({
      id: 'b-late',
      function_id: 'zeta::hook',
      config: { priority: 10, timeout_ms: 5_000, on_error: 'fail_closed' },
    });
    addPreTriggerBindingForTests({
      id: 'b-tie-2',
      function_id: 'beta::hook',
      config: { timeout_ms: 5_000, on_error: 'fail_closed' },
    });
    addPreTriggerBindingForTests({
      id: 'b-tie-1',
      function_id: 'alpha::hook',
      config: { timeout_ms: 5_000, on_error: 'fail_closed' },
    });

    expect(snapshotBindings('shell::run').map((b) => b.function_id)).toEqual([
      'alpha::hook',
      'beta::hook',
      'zeta::hook',
    ]);
  });
});
