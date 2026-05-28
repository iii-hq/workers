import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { isHumanOnlyApprovalFunction } from '../../src/approval-gate/settings/human-only.js';
import { addAlwaysAllow } from '../../src/approval-gate/settings/add-always-allow.js';
import { approveAlways } from '../../src/approval-gate/settings/approve-always.js';
import { removeAlwaysAllow } from '../../src/approval-gate/settings/remove-always-allow.js';
import { setMode } from '../../src/approval-gate/settings/set-mode.js';
import { readSettings } from '../../src/approval-gate/settings/store.js';
import { SETTINGS_STATE_SCOPE } from '../../src/approval-gate/schemas.js';

interface TriggerCall {
  function_id: string;
  payload: { scope: string; key: string; value?: unknown };
}

function makeIii(initial: unknown = null) {
  const store = new Map<string, unknown>();
  if (initial !== null) store.set('sess-1', initial);
  const calls: TriggerCall[] = [];
  const trigger = vi.fn(async (req: TriggerCall) => {
    calls.push(req);
    if (req.function_id === 'state::get') {
      return store.get(req.payload.key) ?? null;
    }
    if (req.function_id === 'state::set') {
      if (req.payload.value === null) store.delete(req.payload.key);
      else store.set(req.payload.key, req.payload.value);
      return null;
    }
    throw new Error(`unexpected trigger ${req.function_id}`);
  });
  return { iii: { trigger } as unknown as ISdk, calls, store };
}

describe('approval-gate settings', () => {
  it('readSettings returns defaults when nothing persisted', async () => {
    const { iii } = makeIii();
    const s = await readSettings(iii, 'sess-1');
    expect(s.mode).toBe('manual');
    expect(s.always_allow).toEqual([]);
    expect(s.mode_set_at).toBe(0);
  });

  it('setMode persists with mode_set_at > 0', async () => {
    const { iii, store } = makeIii();
    const result = await setMode(iii, 'sess-1', 'auto');
    expect(result.mode).toBe('auto');
    expect(result.mode_set_at).toBeGreaterThan(0);
    expect(store.get('sess-1')).toMatchObject({ mode: 'auto' });
  });

  it('addAlwaysAllow is idempotent on function_id', async () => {
    const { iii } = makeIii();
    const once = await addAlwaysAllow(iii, 'sess-1', 'shell::fs::ls');
    expect(once.always_allow).toHaveLength(1);
    const twice = await addAlwaysAllow(iii, 'sess-1', 'shell::fs::ls');
    expect(twice.always_allow).toHaveLength(1);
    expect(twice.always_allow[0].granted_by).toBe('user_click');
  });

  it('removeAlwaysAllow strips matching entries', async () => {
    const { iii } = makeIii();
    await addAlwaysAllow(iii, 'sess-1', 'shell::fs::ls');
    await addAlwaysAllow(iii, 'sess-1', 'fs::read');
    const next = await removeAlwaysAllow(iii, 'sess-1', 'shell::fs::ls');
    expect(next.always_allow.map((e) => e.function_id)).toEqual(['fs::read']);
  });

  it('approveAlways appends to approved_always (idempotent), separate from always_allow', async () => {
    const { iii } = makeIii();
    const once = await approveAlways(iii, 'sess-1', 'shell::exec');
    expect(once.approved_always.map((e) => e.function_id)).toEqual([
      'shell::exec',
    ]);
    expect(once.always_allow).toEqual([]);
    const twice = await approveAlways(iii, 'sess-1', 'shell::exec');
    expect(twice.approved_always).toHaveLength(1);
    expect(twice.approved_always[0].granted_by).toBe('user_click');
  });

  it('approveAlways and addAlwaysAllow write to independent lists', async () => {
    const { iii } = makeIii();
    await addAlwaysAllow(iii, 'sess-1', 'shell::fs::ls');
    const after = await approveAlways(iii, 'sess-1', 'shell::exec');
    expect(after.always_allow.map((e) => e.function_id)).toEqual([
      'shell::fs::ls',
    ]);
    expect(after.approved_always.map((e) => e.function_id)).toEqual([
      'shell::exec',
    ]);
  });

  it('writes go to the SETTINGS_STATE_SCOPE keyed by session_id', async () => {
    const { iii, calls } = makeIii();
    await setMode(iii, 'sess-1', 'full');
    const write = calls.find((c) => c.function_id === 'state::set');
    expect(write?.payload.scope).toBe(SETTINGS_STATE_SCOPE);
    expect(write?.payload.key).toBe('sess-1');
  });

  it('isHumanOnlyApprovalFunction catches every settings handler id', () => {
    expect(isHumanOnlyApprovalFunction('approval::set_mode')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::add_always_allow')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::remove_always_allow')).toBe(
      true,
    );
    expect(isHumanOnlyApprovalFunction('approval::approve_always')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::get_settings')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::clear_settings')).toBe(true);
  });

  it('isHumanOnlyApprovalFunction does NOT block approval::resolve', () => {
    expect(isHumanOnlyApprovalFunction('approval::resolve')).toBe(false);
    expect(isHumanOnlyApprovalFunction('shell::exec')).toBe(false);
  });
});
