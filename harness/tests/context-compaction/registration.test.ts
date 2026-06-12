import { describe, expect, it, vi } from 'vitest';
import { register } from '../../src/context-compaction/register.js';
import type { ISdk } from '../../src/runtime/iii.js';

describe('context-compaction registration', () => {
  it('registers on_turn_end as a function and no stream trigger (woken by a queue message)', async () => {
    const registerFunction = vi.fn();
    const registerTrigger = vi.fn();
    const iii = {
      registerFunction,
      registerTrigger,
      trigger: vi.fn(async () => null),
    } as unknown as ISdk;

    await register(iii);

    // Compaction is woken by a turn-orchestrator queue message, not a stream.
    expect(registerTrigger).not.toHaveBeenCalled();

    const fns = registerFunction.mock.calls.map((c) => c[0] as string);
    expect(fns).toContain('context-compaction::on_turn_end');
    expect(fns).not.toContain('context-compaction::on_agent_event');
  });
});
