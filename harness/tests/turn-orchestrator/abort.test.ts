import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { performAbortSideEffects } from '../../src/turn-orchestrator/abort.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

describe('performAbortSideEffects', () => {
  it('sets the abort_signal flag', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(null);

    await performAbortSideEffects(iii, 's1');

    const setCalls = triggers.filter((t) => t.function_id === 'state::set');
    expect(
      setCalls.some(
        (c) => (c.payload as Record<string, unknown>).key === 'session/s1/abort_signal',
      ),
    ).toBe(true);
  });

  it('skips approval cleanup when record state is not function_awaiting_approval', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);

    await performAbortSideEffects(iii, 's1');

    const approvalWrites = triggers
      .filter((t) => t.function_id === 'state::set')
      .map((t) => t.payload as Record<string, unknown>)
      .filter((p) => p.scope === 'approvals');
    expect(approvalWrites).toHaveLength(0);
    expect(triggers.some((t) => t.function_id === 'approval::sweep_session')).toBe(false);
  });

  it('invokes resume fns with aborted decision when paused on approval', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_awaiting_approval';
    rec.awaiting_approval = [
      { function_call_id: 'fc-1', function_id: 'shell::run', args: {} },
      { function_call_id: 'fc-2', function_id: 'shell::run', args: {} },
    ];
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);

    await performAbortSideEffects(iii, 's1');

    const decisionWrites = triggers
      .filter((t) => t.function_id === 'state::set')
      .map((t) => t.payload as { scope?: string; key?: string; value?: unknown })
      .filter((p) => p.scope === 'approvals');
    expect(decisionWrites.map((p) => p.key).sort()).toEqual(['s1/fc-1', 's1/fc-2']);
    for (const w of decisionWrites) {
      expect(w.value).toEqual({ decision: 'aborted', reason: 'session_aborted' });
    }
    expect(triggers.some((t) => t.function_id.startsWith('turn::approval_resume'))).toBe(false);

    const publishes = triggers.filter((t) => t.function_id === 'iii::durable::publish');
    expect(publishes).toHaveLength(0);
  });
});
