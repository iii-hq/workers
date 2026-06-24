/**
 * localStorage persistence for UI affordances ONLY. Conversation transcripts
 * live in the session-manager worker (see hooks/use-conversations.ts); the
 * legacy `iii-chat-conversations` blob is no longer read or written.
 */

const ACTIVE_KEY = 'iii-chat-active'
const LAST_MODEL_KEY = 'iii-chat-last-model'
const LAST_WORKING_DIR_KEY = 'iii-chat-last-working-dir'
const DEFAULT_PERMISSION_MODE_KEY = 'iii-default-permission-mode'

export type PermissionMode = 'manual' | 'auto' | 'full'
const PERMISSION_MODES: ReadonlySet<PermissionMode> = new Set([
  'manual',
  'auto',
  'full',
])

function isPermissionMode(v: unknown): v is PermissionMode {
  return typeof v === 'string' && PERMISSION_MODES.has(v as PermissionMode)
}

/** User-level default mode applied to NEW conversations only. */
export function loadDefaultPermissionMode(): PermissionMode {
  try {
    const raw = localStorage.getItem(DEFAULT_PERMISSION_MODE_KEY)
    return isPermissionMode(raw) ? raw : 'manual'
  } catch {
    return 'manual'
  }
}

export function saveDefaultPermissionMode(mode: PermissionMode): void {
  try {
    localStorage.setItem(DEFAULT_PERMISSION_MODE_KEY, mode)
  } catch {
    /* best-effort */
  }
}

const DEFAULT_ALLOWLIST_KEY = 'iii-default-allowlist'

/** User-level allowlist used to seed new conversations' backend state. */
export function loadDefaultAllowlist(): string[] {
  try {
    const raw = localStorage.getItem(DEFAULT_ALLOWLIST_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (v): v is string => typeof v === 'string' && v.length > 0,
    )
  } catch {
    return []
  }
}

export function saveDefaultAllowlist(list: string[]): void {
  try {
    /* Stable insertion order matters for human review; sort once on write
     * so the list reads consistently across sessions. */
    const unique = Array.from(new Set(list)).sort()
    localStorage.setItem(DEFAULT_ALLOWLIST_KEY, JSON.stringify(unique))
  } catch {
    /* best-effort */
  }
}

export function loadActiveId(): string | null {
  try {
    return localStorage.getItem(ACTIVE_KEY)
  } catch {
    return null
  }
}

export function saveActiveId(id: string | null): void {
  try {
    if (id) localStorage.setItem(ACTIVE_KEY, id)
    else localStorage.removeItem(ACTIVE_KEY)
  } catch {
    /* best-effort */
  }
}

export function loadLastModel(): string | null {
  try {
    return localStorage.getItem(LAST_MODEL_KEY)
  } catch {
    return null
  }
}

export function saveLastModel(id: string | null): void {
  try {
    if (id) localStorage.setItem(LAST_MODEL_KEY, id)
    else localStorage.removeItem(LAST_MODEL_KEY)
  } catch {
    /* best-effort */
  }
}

/**
 * Most recently chosen working directory, pre-selected on the next new chat so
 * repeat sessions in the same project are one confirm, not a re-browse. A
 * convenience default only — the picker still lets the user browse and override.
 */
export function loadLastWorkingDir(): string | null {
  try {
    return localStorage.getItem(LAST_WORKING_DIR_KEY)
  } catch {
    return null
  }
}

export function saveLastWorkingDir(dir: string | null): void {
  try {
    if (dir) localStorage.setItem(LAST_WORKING_DIR_KEY, dir)
    else localStorage.removeItem(LAST_WORKING_DIR_KEY)
  } catch {
    /* best-effort */
  }
}
