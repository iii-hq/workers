import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { isHumanOnlyApprovalFunction } from '../../src/approval-gate/settings/human-only.js';
import { addAlwaysAllow } from '../../src/approval-gate/settings/add-always-allow.js';
import { approveAlways } from '../../src/approval-gate/settings/approve-always.js';
import { removeAlwaysAllow } from '../../src/approval-gate/settings/remove-always-allow.js';
import { setMode } from '../../src/approval-gate/settings/set-mode.js';
import { clearSettings, readSettings } from '../../src/approval-gate/settings/store.js';
import { SETTINGS_STATE_SCOPE } from '../../src/approval-gate/schemas.js';

interface TriggerCall {
  function_id: string;
  payload: {
    scope: string;
    key: string;
    value?: unknown;
    ops?: Array<{ type: string; path?: string; value?: unknown }>;
  };
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
    if (req.function_id === 'state::delete') {
      const old_value = store.get(req.payload.key) ?? null;
      store.delete(req.payload.key);
      return { old_value };
    }
    // Atomic field-scoped update (synchronous read-modify-write) modelling the
    // `set` and `append` ops the settings mutations use.
    if (req.function_id === 'state::update') {
      const old_value = store.get(req.payload.key) ?? null;
      const rec: Record<string, unknown> =
        old_value && typeof old_value === 'object' && !Array.isArray(old_value)
          ? { ...(old_value as Record<string, unknown>) }
          : {};
      for (const op of req.payload.ops ?? []) {
        const path = op.path ?? '';
        if (op.type === 'set') rec[path] = op.value;
        else if (op.type === 'append') {
          const arr = Array.isArray(rec[path]) ? (rec[path] as unknown[]) : [];
          rec[path] = [...arr, op.value];
        }
      }
      store.set(req.payload.key, rec);
      return { old_value, new_value: rec };
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

  it('clearSettings deletes the key (no null tombstone) and reverts to defaults', async () => {
    const { iii, store, calls } = makeIii();
    await setMode(iii, 'sess-1', 'auto');
    expect(store.has('sess-1')).toBe(true);

    await clearSettings(iii, 'sess-1');

    expect(store.has('sess-1')).toBe(false);
    expect(calls.some((c) => c.function_id === 'state::delete')).toBe(true);
    const s = await readSettings(iii, 'sess-1');
    expect(s.mode).toBe('manual');
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
    expect(once.approved_always.map((e) => e.function_id)).toEqual(['shell::exec']);
    expect(once.always_allow).toEqual([]);
    const twice = await approveAlways(iii, 'sess-1', 'shell::exec');
    expect(twice.approved_always).toHaveLength(1);
    expect(twice.approved_always[0].granted_by).toBe('user_click');
  });

  it('approveAlways and addAlwaysAllow write to independent lists', async () => {
    const { iii } = makeIii();
    await addAlwaysAllow(iii, 'sess-1', 'shell::fs::ls');
    const after = await approveAlways(iii, 'sess-1', 'shell::exec');
    expect(after.always_allow.map((e) => e.function_id)).toEqual(['shell::fs::ls']);
    expect(after.approved_always.map((e) => e.function_id)).toEqual(['shell::exec']);
  });

  it('writes go to the SETTINGS_STATE_SCOPE keyed by session_id', async () => {
    const { iii, calls } = makeIii();
    await setMode(iii, 'sess-1', 'full');
    const write = calls.find(
      (c) => c.function_id === 'state::set' || c.function_id === 'state::update',
    );
    expect(write?.payload.scope).toBe(SETTINGS_STATE_SCOPE);
    expect(write?.payload.key).toBe('sess-1');
  });

  // --- Atomic field-scoped mutations: disjoint fields must not clobber. ---

  it('concurrent setMode and addAlwaysAllow both persist (no lost update)', async () => {
    const { iii } = makeIii();
    await Promise.all([
      setMode(iii, 'sess-1', 'auto'),
      addAlwaysAllow(iii, 'sess-1', 'shell::exec'),
    ]);
    const s = await readSettings(iii, 'sess-1');
    expect(s.mode).toBe('auto');
    expect(s.always_allow.map((e) => e.function_id)).toEqual(['shell::exec']);
  });

  it('concurrent setMode and approveAlways both persist (no lost update)', async () => {
    const { iii } = makeIii();
    await Promise.all([
      setMode(iii, 'sess-1', 'full'),
      approveAlways(iii, 'sess-1', 'shell::exec'),
    ]);
    const s = await readSettings(iii, 'sess-1');
    expect(s.mode).toBe('full');
    expect(s.approved_always.map((e) => e.function_id)).toEqual(['shell::exec']);
  });

  it('concurrent adds of different ids both land (append composition)', async () => {
    const { iii } = makeIii();
    await Promise.all([
      addAlwaysAllow(iii, 'sess-1', 'a::one'),
      addAlwaysAllow(iii, 'sess-1', 'b::two'),
    ]);
    const s = await readSettings(iii, 'sess-1');
    expect(s.always_allow.map((e) => e.function_id).sort()).toEqual(['a::one', 'b::two']);
  });

  it('readSettings backfills a partial record and keeps the configured default mode', async () => {
    // A field-scoped append can persist only { always_allow }; the read must
    // still produce a complete, valid record (not fall back to all-defaults).
    const { iii } = makeIii({
      always_allow: [{ function_id: 'shell::exec', granted_at: 1, granted_by: 'user_click' }],
    });
    const s = await readSettings(iii, 'sess-1');
    expect(s.always_allow.map((e) => e.function_id)).toEqual(['shell::exec']);
    expect(s.mode).toBe('manual');
    expect(s.approved_always).toEqual([]);
    expect(s.mode_set_at).toBe(0);
  });

  it('isHumanOnlyApprovalFunction catches every settings handler id', () => {
    expect(isHumanOnlyApprovalFunction('approval::set_mode')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::add_always_allow')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::remove_always_allow')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::approve_always')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::get_settings')).toBe(true);
    expect(isHumanOnlyApprovalFunction('approval::clear_settings')).toBe(true);
  });

  it('isHumanOnlyApprovalFunction does NOT block approval::resolve', () => {
    expect(isHumanOnlyApprovalFunction('approval::resolve')).toBe(false);
    expect(isHumanOnlyApprovalFunction('shell::exec')).toBe(false);
  });
});
