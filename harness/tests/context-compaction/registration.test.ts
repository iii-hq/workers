import { describe, expect, it, vi } from 'vitest';
import { register } from '../../src/context-compaction/register.js';
import type { ISdk } from '../../src/runtime/iii.js';

describe('context-compaction registration', () => {
  it('registers only compact_session (the thin /compact wrapper) and no triggers', async () => {
    const registerFunction = vi.fn();
    const registerTrigger = vi.fn();
    const iii = {
      registerFunction,
      registerTrigger,
      trigger: vi.fn(async () => null),
    } as unknown as ISdk;

    await register(iii);

    // The async post-turn compactor and its wakes are retired — context-manager
    // owns assemble/compact, and the harness compacts inline on the hot path.
    expect(registerTrigger).not.toHaveBeenCalled();

    const fns = registerFunction.mock.calls.map((c) => c[0] as string);
    expect(fns).toEqual(['context-compaction::compact_session']);
    expect(fns).not.toContain('context-compaction::on_turn_end');
    expect(fns).not.toContain('context-compaction::compact_now');
    expect(fns).not.toContain('context-compaction::prune_tool_outputs');
  });
});
