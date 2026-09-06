/**
 * Session working-directory plumbing shared by the picker and ChatView:
 * the shell workspace control-plane function ids, error unwrapping, live
 * validation, and the stack's default working directory (the folder the
 * harness was launched from — `harness::filesystem::info` — which new
 * chats are pre-filled with so files land where the user is working).
 */

import { getIiiClient } from '@/lib/iii-client'
import type { WorkingDirScope } from '@/types/chat'

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

function errCode(err: unknown): string | undefined {
  if (err && typeof err === 'object' && 'code' in err) {
    const code = String((err as { code: unknown }).code)
    if (/^[A-Z]\d{3}$/.test(code)) return code
  }
  const raw =
    err instanceof Error
      ? err.message
      : err && typeof err === 'object' && 'message' in err
        ? String((err as { message: unknown }).message)
        : String(err)
  const nested = [...raw.matchAll(/"code"\s*:\s*"([A-Z]\d{3})"/g)]
  return nested.at(-1)?.[1] ?? raw.match(/\b([A-Z]\d{3})\b/)?.[1]
}

export type WorkspaceValidation =
  | { ok: true; path: string }
  | { ok: false; error: string; code?: string }

export type WorkingDirActivation =
  | { status: 'valid'; path: string }
  | { status: 'recovered'; path: string | null }
  | { status: 'unavailable'; path: string }

export function workingDirRecoveryNotice(
  savedDir: string,
  nextDir: string | null,
): string {
  return nextDir === null
    ? `working directory ${savedDir} is no longer available; this session is now unscoped — applies to the messages that follow`
    : `working directory changed to ${nextDir} because ${savedDir} is no longer available — applies to the messages that follow`
}

/**
 * The transcript marker for a scope change: the plain sentence (exports,
 * announcements, plain-text fallbacks) plus the structured scope the
 * working-dir row renders from. One builder for every cause so the picker,
 * the recovery path and the "vanished with no fallback" path all drop the
 * same shape into the conversation.
 */
export function workingDirScopeNotice(scope: WorkingDirScope): {
  content: string
  scope: WorkingDirScope
} {
  const previous = scope.previousPath ?? null
  const content =
    scope.cause === 'selected'
      ? scope.path === null
        ? 'working directory cleared; this session is now unscoped — applies to the messages that follow'
        : `working directory changed to ${scope.path} — applies to the messages that follow`
      : workingDirRecoveryNotice(previous ?? '(unknown)', scope.path)
  return { content, scope }
}

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
    const code = errCode(err)
    return {
      ok: false,
      error: errMsg(err),
      ...(code ? { code } : {}),
    }
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
  const result = await resolveLiveDefaultWorkingDir()
  return result.status === 'resolved' ? result.path : null
}

type DefaultWorkingDirResolution =
  | { status: 'resolved'; path: string | null }
  | { status: 'unavailable' }

async function resolveLiveDefaultWorkingDir(): Promise<DefaultWorkingDirResolution> {
  try {
    const client = await getIiiClient()
    const info = await client.trigger<HarnessFilesystemInfoResult>(
      HARNESS_FILESYSTEM_INFO_FUNCTION_ID,
      {},
    )
    const root = info?.default_root
    if (typeof root !== 'string' || root.length === 0) {
      return { status: 'resolved', path: null }
    }
    const validated = await validateWorkspaceDir(root)
    if (validated.ok) return { status: 'resolved', path: validated.path }
    return isInvalidWorkspace(validated)
      ? { status: 'resolved', path: null }
      : { status: 'unavailable' }
  } catch {
    return { status: 'unavailable' }
  }
}

function isInvalidWorkspace(
  result: Extract<WorkspaceValidation, { ok: false }>,
): boolean {
  if (
    result.code === 'S210' ||
    result.code === 'S211' ||
    result.code === 'S212'
  ) {
    return true
  }
  return /not found or not accessible|not a directory/i.test(result.error)
}

/**
 * Reconcile persisted session scope with the live filesystem. A missing saved
 * path is normal for temporary Harness projects, so recover to the current
 * Harness default instead of turning it into a blocking picker error. Resolve
 * the default live here rather than trusting the page cache: the filesystem
 * may have changed since the picker was first rendered.
 */
export async function activateWorkingDir(
  savedDir: string,
): Promise<WorkingDirActivation> {
  const saved = await validateWorkspaceDir(savedDir)
  if (saved.ok) return { status: 'valid', path: saved.path }
  if (!isInvalidWorkspace(saved)) {
    return { status: 'unavailable', path: savedDir }
  }

  const fallback = await resolveLiveDefaultWorkingDir()
  if (fallback.status === 'unavailable') {
    return { status: 'unavailable', path: savedDir }
  }
  defaultWorkingDirPromise = Promise.resolve(fallback.path)
  return { status: 'recovered', path: fallback.path }
}

/** Test hook: drop the page-lifetime cache. */
export function resetDefaultWorkingDirForTests(): void {
  defaultWorkingDirPromise = null
}
