/**
 * The parts of the agent terminal page that decide WHICH session and WHERE,
 * kept free of React and xterm so they can be tested on their own: the lease
 * that reattaches a tab to its running agent (now with the folder it runs
 * in), which folder a fresh session starts in, what the worker's describe
 * function is asked, and which orphaned session may be adopted.
 */

export interface Lease {
  sessionId: string
  reconnectToken: string
  /** The folder the session runs in, as the worker resolved it. */
  cwd?: string
}

export type Call = <T>(fn: string, payload: unknown, timeoutMs?: number) => Promise<T>

/**
 * The lease lives in `localStorage`, not `sessionStorage`: a session outlives
 * the browser tab that opened it. A lease written by an older build to
 * `sessionStorage` is read once and carried over.
 */
export function readLease(prefix: string, tabId: string): Lease | null {
  try {
    const key = prefix + tabId
    const raw = localStorage.getItem(key) ?? sessionStorage.getItem(key)
    if (!raw) return null
    const lease = JSON.parse(raw) as Partial<Lease>
    if (typeof lease?.sessionId !== 'string' || typeof lease.reconnectToken !== 'string') return null
    return {
      sessionId: lease.sessionId,
      reconnectToken: lease.reconnectToken,
      ...(typeof lease.cwd === 'string' && lease.cwd ? { cwd: lease.cwd } : {}),
    }
  } catch {
    return null
  }
}

export function writeLease(prefix: string, tabId: string, lease: Lease | null): void {
  try {
    const key = prefix + tabId
    sessionStorage.removeItem(key)
    if (lease) localStorage.setItem(key, JSON.stringify(lease))
    else localStorage.removeItem(key)
  } catch {
    // Storage full or blocked: the terminal still works, only the reattach
    // across a reload is lost.
  }
}

/**
 * The folder a FRESH session asks for: the one the person picked in this
 * pane, else the chat's working directory, else none (the worker then uses
 * the compose project's folder).
 */
export function startFolder(picked: string | null, workingDir: string | null | undefined): string | null {
  const pick = picked?.trim()
  if (pick) return pick
  const chat = workingDir?.trim()
  return chat || null
}

/**
 * The describe request. A page without the folder pill sends `{}`, exactly
 * as before, so a worker whose describe takes no folder (pi) is unaffected.
 */
export function describePayload(folderPicker: boolean, folder: string | null): Record<string, string> {
  return folderPicker && folder ? { cwd: folder } : {}
}

export interface SessionSummary {
  session_id: string
  cwd?: string
  program: string | null
  status: string | { exited?: unknown }
  /** Where output goes; null while detached. */
  output_function_id?: string | null
  ui: string | null
}

/**
 * The unattached session this page left behind that may be taken back: same
 * page family, same program and, when the page runs sessions in a chosen
 * folder, that folder. Without the folder check a folder change would adopt
 * the session it just left.
 */
export function pickOrphan(
  sessions: SessionSummary[],
  want: { program: string; ui: string; cwd?: string },
): string | null {
  const orphan = sessions.find(
    (s) =>
      s.ui === want.ui &&
      s.status === 'detached' &&
      s.program === want.program &&
      (want.cwd === undefined || s.cwd === want.cwd),
  )
  return orphan?.session_id ?? null
}

/**
 * Move this pane's agent to another folder. Claude Code cannot change folder
 * mid-session, so the running session is CLOSED (not detached: a detached one
 * would linger as an orphan in the old folder) and the lease cleared; the
 * caller then starts a fresh session, which runs the plugin check there.
 */
export async function leaveSession(
  call: Call,
  conn: { sessionId: string; accessKey: string } | null,
  leasePrefix: string,
  tabId: string,
): Promise<void> {
  writeLease(leasePrefix, tabId, null)
  if (!conn) return
  try {
    await call('shell::pty::close', { session_id: conn.sessionId, access_key: conn.accessKey })
  } catch {
    // Already gone: nothing left to close.
  }
}
