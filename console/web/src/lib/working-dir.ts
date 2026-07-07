/**
 * Session working-directory plumbing shared by the picker and ChatView:
 * the shell workspace control-plane function ids, error unwrapping, live
 * validation, and the stack's default working directory (the folder the
 * harness was launched from — `harness::filesystem::info` — which new
 * chats are pre-filled with so files land where the user is working).
 */

import { getIiiClient } from '@/lib/iii-client'

export const WORKSPACE_ROOTS_FUNCTION_ID = 'shell::workspace::roots'
export const WORKSPACE_LIST_FUNCTION_ID = 'shell::workspace::list'
export const WORKSPACE_VALIDATE_FUNCTION_ID = 'shell::workspace::validate'
export const HARNESS_FILESYSTEM_INFO_FUNCTION_ID = 'harness::filesystem::info'

interface WorkspaceValidateResult {
  path: string
}

interface HarnessFilesystemInfoResult {
  default_root?: string | null
}

/**
 * iii triggers reject with a plain object `{ code, message }`, not an Error,
 * and the message is often a nested `handler error: {"code":"C211","message":"…"}`.
 * Pull out the human-readable inner message.
 */
export function errMsg(err: unknown): string {
  const raw =
    err instanceof Error
      ? err.message
      : err && typeof err === 'object' && 'message' in err
        ? String((err as { message: unknown }).message)
        : String(err)
  // Errors nest: `handler error: {"code":"C211","message":"…"}`. Prefer the
  // innermost (last) message and tolerate escaped quotes inside it.
  const matches = [...raw.matchAll(/"message"\s*:\s*"((?:[^"\\]|\\.)*)"/g)]
  if (matches.length === 0) return raw
  return matches[matches.length - 1][1].replace(/\\(.)/g, '$1')
}

export type WorkspaceValidation =
  | { ok: true; path: string }
  | { ok: false; error: string }

/**
 * Validate a directory against the LIVE shell worker. On success the result
 * carries the worker-echoed canonical path — always store that, never the
 * raw input, so paths stay stable across symlinks.
 */
export async function validateWorkspaceDir(
  dir: string,
): Promise<WorkspaceValidation> {
  try {
    const client = await getIiiClient()
    const res = await client.trigger<WorkspaceValidateResult>(
      WORKSPACE_VALIDATE_FUNCTION_ID,
      { path: dir },
    )
    return { ok: true, path: res?.path ?? dir }
  } catch (err) {
    return { ok: false, error: errMsg(err) }
  }
}

let defaultWorkingDirPromise: Promise<string | null> | null = null

/**
 * The working directory a new chat defaults to: the harness-reported default
 * root (its launch folder), accepted only when the live shell validates it.
 * Resolves to `null` when the harness has no default, the folder is invalid
 * on the shell host, or either worker is unreachable — callers then leave the
 * chat unscoped exactly as before. Cached for the page's lifetime (the stack's
 * launch folder doesn't move).
 */
export function fetchDefaultWorkingDir(): Promise<string | null> {
  if (!defaultWorkingDirPromise) {
    defaultWorkingDirPromise = resolveDefaultWorkingDir()
  }
  return defaultWorkingDirPromise
}

async function resolveDefaultWorkingDir(): Promise<string | null> {
  try {
    const client = await getIiiClient()
    const info = await client.trigger<HarnessFilesystemInfoResult>(
      HARNESS_FILESYSTEM_INFO_FUNCTION_ID,
      {},
    )
    const root = info?.default_root
    if (typeof root !== 'string' || root.length === 0) return null
    const validated = await validateWorkspaceDir(root)
    return validated.ok ? validated.path : null
  } catch {
    return null
  }
}

/** Test hook: drop the page-lifetime cache. */
export function resetDefaultWorkingDirForTests(): void {
  defaultWorkingDirPromise = null
}
