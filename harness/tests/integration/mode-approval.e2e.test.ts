/**
 * Mode-driven approval e2e. Unlike the parallel-approval suite, these
 * tests do NOT mock `dispatchWithHook` — they run the real `consultBefore`
 * against per-session `approval_settings` seeded into the harness store,
 * exercising the full ordering (full > approved_always > always_allow >
 * yaml policy) through the orchestrator's function_execute path.
 */

import { describe, expect, it } from 'vitest';
import { SETTINGS_STATE_SCOPE } from '../../src/approval-gate/schemas.js';
import {
  createParallelApprovalHarness,
  executionEvents,
  makeAssistantWithCalls,
  type ParallelApprovalHarness,
} from './parallel-approval-harness.js';

type PermissionMode = 'manual' | 'auto' | 'full';

interface SeedSettings {
  mode?: PermissionMode;
  always_allow?: string[];
  approved_always?: string[];
}

function entry(function_id: string) {
  return { function_id, granted_at: 1, granted_by: 'user_click' as const };
}

/** Persist approval_settings into the harness store before runExecute. */
function seedSettings(h: ParallelApprovalHarness, session_id: string, s: SeedSettings): void {
  h.stateStore.set(`${SETTINGS_STATE_SCOPE}/${session_id}`, {
    mode: s.mode ?? 'manual',
    always_allow: (s.always_allow ?? []).map(entry),
    approved_always: (s.approved_always ?? []).map(entry),
    mode_set_at: s.mode ? 1 : 0,
  });
}

function awaitingIds(h: ParallelApprovalHarness, session_id: string): string[] {
  return h.loadTurnRecord(session_id)?.awaiting_approval?.map((e) => e.function_call_id) ?? [];
}

describe('mode-driven approval e2e (real consultBefore)', () => {
  it('full mode executes the call immediately without parking or writing an approval', async () => {
    const h = createParallelApprovalHarness();
    seedSettings(h, 'sess-full', { mode: 'full' });
    h.seedExecute('sess-full', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));

    await h.runExecute('sess-full');

    const rec = h.loadTurnRecord('sess-full');
    expect(rec?.state).toBe('assistant_streaming');
    expect(rec?.awaiting_approval).toEqual([]);
    expect(rec?.work).toBeUndefined();
    expect(h.stateStore.has('approvals/sess-full/fc-1')).toBe(false);
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-1')).toHaveLength(1);
  });

  it('approved_always honors the grant in MANUAL mode (executes without parking)', async () => {
    const h = createParallelApprovalHarness();
    seedSettings(h, 'sess-grant', {
      mode: 'manual',
      approved_always: ['shell::run'],
    });
    h.seedExecute('sess-grant', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));

    await h.runExecute('sess-grant');

    const rec = h.loadTurnRecord('sess-grant');
    expect(rec?.state).toBe('assistant_streaming');
    expect(rec?.awaiting_approval).toEqual([]);
    expect(executionEvents(h.emitted, 'function_execution_end', 'fc-1')).toHaveLength(1);
  });

  it('auto mode + always_allow hit executes without parking', async () => {
    const h = createParallelApprovalHarness();
    seedSettings(h, 'sess-auto', {
      mode: 'auto',
      always_allow: ['shell::run'],
    });
    h.seedExecute('sess-auto', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));

    await h.runExecute('sess-auto');

    expect(awaitingIds(h, 'sess-auto')).toEqual([]);
    expect(h.loadTurnRecord('sess-auto')?.state).toBe('assistant_streaming');
  });

  it('manual mode with no grant parks the call (falls through to policy → needs_approval)', async () => {
    const h = createParallelApprovalHarness();
    seedSettings(h, 'sess-manual', { mode: 'manual' });
    h.seedExecute(
      'sess-manual',
      makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]),
    );

    await h.runExecute('sess-manual');

    expect(h.loadTurnRecord('sess-manual')?.state).toBe('function_awaiting_approval');
    expect(awaitingIds(h, 'sess-manual')).toEqual(['fc-1']);
  });

  it('always_allow is DORMANT in manual mode — the call still parks', async () => {
    const h = createParallelApprovalHarness();
    seedSettings(h, 'sess-dormant', {
      mode: 'manual',
      always_allow: ['shell::run'],
    });
    h.seedExecute(
      'sess-dormant',
      makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]),
    );

    await h.runExecute('sess-dormant');

    expect(h.loadTurnRecord('sess-dormant')?.state).toBe('function_awaiting_approval');
    expect(awaitingIds(h, 'sess-dormant')).toEqual(['fc-1']);
  });

  it('auto mode parks a call whose id is not on the allowlist', async () => {
    const h = createParallelApprovalHarness();
    seedSettings(h, 'sess-miss', {
      mode: 'auto',
      always_allow: ['other::fn'],
    });
    h.seedExecute('sess-miss', makeAssistantWithCalls([{ id: 'fc-1', functionId: 'shell::run' }]));

    await h.runExecute('sess-miss');

    expect(awaitingIds(h, 'sess-miss')).toEqual(['fc-1']);
  });

  it('denies an agent attempt to call a human-only approval function (self-escalation)', async () => {
    const h = createParallelApprovalHarness();
    seedSettings(h, 'sess-escalate', { mode: 'manual' });
    h.seedExecute(
      'sess-escalate',
      makeAssistantWithCalls([
        { id: 'fc-1', functionId: 'approval::set_mode', payload: { mode: 'full' } },
      ]),
    );

    await h.runExecute('sess-escalate');

    const rec = h.loadTurnRecord('sess-escalate');
    // Denied up front: not parked, not executed, batch finalizes with an
    // error result carrying the human_only_function denial. The result
    // travels on turn_end; the inline resume clears the record.
    expect(rec?.awaiting_approval).toEqual([]);
    expect(rec?.state).toBe('assistant_streaming');
    expect(rec?.function_results).toEqual([]);
    const turnEnd = h.emitted.find((e) => e.type === 'turn_end') as
      | {
          function_results: Array<{
            function_call_id: string;
            is_error: boolean;
            details: unknown;
          }>;
        }
      | undefined;
    const denied = turnEnd?.function_results.find((r) => r.function_call_id === 'fc-1');
    expect(denied?.is_error).toBe(true);
    expect(JSON.stringify(denied?.details)).toContain('human_only_function');
    // No real mode change leaked into settings.
    const settings = h.stateStore.get(`${SETTINGS_STATE_SCOPE}/sess-escalate`) as {
      mode: string;
    };
    expect(settings.mode).toBe('manual');
  });
});
