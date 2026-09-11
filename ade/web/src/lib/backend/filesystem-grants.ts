/**
 * RPC adapters for the harness's per-session filesystem grant control plane
 * (`harness::filesystem::grants` / `harness::filesystem::revoke`). These are
 * user-initiated, read/revoke only: granting happens as a side effect of
 * `approval::resolve { access_duration }` on the gate, not here.
 *
 * Both functions are registered off the model catalog by the harness.
 */

import { getIiiClient } from '@/lib/iii-client'

interface FilesystemGrantsResponse {
  session_id: string
  roots: string[]
}

export interface ListGrantedDirsResult {
  dirs: string[]
  /** False when `harness::filesystem::grants` isn't registered. */
  supported: boolean
}

function coerceRoots(raw: unknown): string[] {
  const roots = (raw as Partial<FilesystemGrantsResponse> | null)?.roots
  return Array.isArray(roots)
    ? roots.filter((r): r is string => typeof r === 'string')
    : []
}

export async function listGrantedDirs(
  sessionId: string,
): Promise<ListGrantedDirsResult> {
  try {
    const client = await getIiiClient()
    const raw = await client.trigger<FilesystemGrantsResponse>(
      'harness::filesystem::grants',
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
  const raw = await client.trigger<FilesystemGrantsResponse>(
    'harness::filesystem::revoke',
    { session_id: sessionId, root },
  )
  return coerceRoots(raw)
}
