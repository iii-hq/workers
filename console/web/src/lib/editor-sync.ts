/**
 * Keep the shared editor workspace pointed at the chat's working directory.
 *
 * The editor worker holds one active root (the workspace every surface sees);
 * chat holds a per-conversation working directory. Without this bridge the
 * editor page keeps showing whatever was opened last — usually the engine's
 * own folder — while the conversation works somewhere else entirely.
 *
 * Best-effort on purpose: the editor keeps per-root sessions, so repointing
 * is lossless, and when the editor worker is not installed the call fails
 * quietly and chat behaves exactly as before.
 */
import { getIiiClient } from './iii-client'

const OPEN_FUNCTION_ID = 'editor::workspace::open'

type TriggerFn = (
  functionId: string,
  payload: Record<string, unknown>,
) => Promise<unknown>

/**
 * Consecutive-call dedupe: activating the same conversation (or re-validating
 * the same folder) must not re-open the workspace it already points at.
 */
let lastSyncedRoot: string | null = null

export async function syncEditorWorkspace(
  root: string,
  trigger?: TriggerFn,
): Promise<boolean> {
  if (!root || root === lastSyncedRoot) return false
  const call =
    trigger ??
    (async (functionId: string, payload: Record<string, unknown>) => {
      const client = await getIiiClient()
      return client.trigger(functionId, payload, { timeoutMs: 15_000 })
    })
  try {
    await call(OPEN_FUNCTION_ID, { root })
    lastSyncedRoot = root
    return true
  } catch {
    // Editor worker absent, or the folder is not reachable from it. The
    // conversation is unaffected either way; the next root change retries.
    return false
  }
}

export function resetEditorSyncForTests(): void {
  lastSyncedRoot = null
}
