import { describe, expect, it, vi } from 'vitest';
import { register } from '../../src/context-compaction/register.js';
import type { ISdk } from '../../src/runtime/iii.js';

describe('context-compaction stream subscription', () => {
  it('subscribes to agent::turn_end, not the full agent::events firehose', async () => {
    const registerTrigger = vi.fn();
    const iii = {
      registerFunction: vi.fn(),
      registerTrigger,
      trigger: vi.fn(async () => null),
    } as unknown as ISdk;

    await register(iii);

    const streamTriggers = registerTrigger.mock.calls
      .map((c) => c[0] as { type?: string; function_id?: string; config?: { stream_name?: string } })
      .filter(
        (t) => t?.type === 'stream' && t?.function_id === 'context-compaction::on_agent_event',
      );

    expect(streamTriggers).toHaveLength(1);
    expect(streamTriggers[0].config?.stream_name).toBe('agent::turn_end');
  });
});
