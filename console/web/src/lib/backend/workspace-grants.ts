/**
 * RPC adapters for the harness's per-session workspace grant control plane
 * (`harness::workspace::grants` / `harness::workspace::revoke` — see
 * contracts.md "Workspace grant control-plane functions"). These are
 * user-initiated, read/revoke only: granting happens as a side effect of
 * `approval::resolve { grant_scope }` on the gate, not here.
 *
 * Both functions are registered off the model catalog by the harness, but
 * only when the harness build includes the folder-access-grants feature.
 * Older harnesses reject with "function not found" — callers feature-detect
 * by call failure and hide the session-grants UI group.
 */

import { getIiiClient } from '@/lib/iii-client'

interface WorkspaceGrantsResponse {
  session_id: string
  roots: string[]
}

export interface ListGrantedDirsResult {
  dirs: string[]
  /** False when `harness::workspace::grants` isn't registered. */
  supported: boolean
}

function coerceRoots(raw: unknown): string[] {
  const roots = (raw as Partial<WorkspaceGrantsResponse> | null)?.roots
  return Array.isArray(roots)
    ? roots.filter((r): r is string => typeof r === 'string')
    : []
}

export async function listGrantedDirs(
  sessionId: string,
): Promise<ListGrantedDirsResult> {
  try {
    const client = await getIiiClient()
    const raw = await client.trigger<WorkspaceGrantsResponse>(
      'harness::workspace::grants',
      { session_id: sessionId },
    )
    return { dirs: coerceRoots(raw), supported: true }
  } catch {
    return { dirs: [], supported: false }
  }
}

export async function revokeDir(
  sessionId: string,
  root: string,
): Promise<string[]> {
  const client = await getIiiClient()
  const raw = await client.trigger<WorkspaceGrantsResponse>(
    'harness::workspace::revoke',
    { session_id: sessionId, root },
  )
  return coerceRoots(raw)
}
