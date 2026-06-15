/**
 * RPC adapters for the per-session approval settings (`approval::*`
 * functions on the standalone approval-gate worker). These are
 * user-initiated RPCs only — agent function calls cannot reach them
 * (the gate hard-denies approval::* function ids).
 */

import { getIiiClient } from '@/lib/iii-client'

export type PermissionMode = 'manual' | 'auto' | 'full'

export interface AlwaysAllowEntry {
  function_id: string
  granted_at: number
  granted_by: 'user_click' | 'seed'
}

export interface ApprovalSettings {
  mode: PermissionMode
  always_allow: AlwaysAllowEntry[]
  approved_always: AlwaysAllowEntry[]
  mode_set_at: number
}

export const DEFAULT_APPROVAL_SETTINGS: ApprovalSettings = {
  mode: 'manual',
  always_allow: [],
  approved_always: [],
  mode_set_at: 0,
}

function coerceEntries(raw: unknown): AlwaysAllowEntry[] {
  const list = Array.isArray(raw) ? raw : []
  return list
    .filter(
      (entry): entry is Record<string, unknown> =>
        !!entry && typeof entry === 'object',
    )
    .map(
      (entry): AlwaysAllowEntry => ({
        function_id: String(entry.function_id ?? ''),
        granted_at: Number(entry.granted_at ?? 0),
        granted_by: entry.granted_by === 'seed' ? 'seed' : 'user_click',
      }),
    )
    .filter((entry) => entry.function_id.length > 0)
}

function coerceSettings(raw: unknown): ApprovalSettings {
  if (!raw || typeof raw !== 'object') return DEFAULT_APPROVAL_SETTINGS
  // The approval-gate worker wraps the record: mutations return
  // { settings }, get_settings returns { settings, source }.
  const outer = raw as Record<string, unknown>
  const r = (
    outer.settings && typeof outer.settings === 'object'
      ? outer.settings
      : outer
  ) as Record<string, unknown>
  const mode: PermissionMode =
    r.mode === 'auto' || r.mode === 'full' ? r.mode : 'manual'
  return {
    mode,
    always_allow: coerceEntries(r.always_allow),
    approved_always: coerceEntries(r.approved_always),
    mode_set_at: typeof r.mode_set_at === 'number' ? r.mode_set_at : 0,
  }
}

export async function getApprovalSettings(
  sessionId: string,
): Promise<ApprovalSettings> {
  const client = await getIiiClient()
  const raw = await client.call('approval::get-settings', {
    session_id: sessionId,
  })
  return coerceSettings(raw)
}

export async function setApprovalMode(
  sessionId: string,
  mode: PermissionMode,
): Promise<ApprovalSettings> {
  const client = await getIiiClient()
  const raw = await client.call('approval::set-mode', {
    session_id: sessionId,
    mode,
  })
  return coerceSettings(raw)
}

export async function addAlwaysAllow(
  sessionId: string,
  functionId: string,
): Promise<ApprovalSettings> {
  const client = await getIiiClient()
  const raw = await client.call('approval::add-always-allow', {
    session_id: sessionId,
    function_id: functionId,
  })
  return coerceSettings(raw)
}

export async function removeAlwaysAllow(
  sessionId: string,
  functionId: string,
): Promise<ApprovalSettings> {
  const client = await getIiiClient()
  const raw = await client.call('approval::remove-always-allow', {
    session_id: sessionId,
    function_id: functionId,
  })
  return coerceSettings(raw)
}

export async function approveAlways(
  sessionId: string,
  functionId: string,
): Promise<ApprovalSettings> {
  const client = await getIiiClient()
  const raw = await client.call('approval::approve-always', {
    session_id: sessionId,
    function_id: functionId,
  })
  return coerceSettings(raw)
}

export async function clearApprovalSettings(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await client
    .call('approval::clear-settings', { session_id: sessionId })
    .catch(() => {
      /* best-effort cleanup on conversation deletion */
    })
}
