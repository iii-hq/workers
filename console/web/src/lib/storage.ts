/**
 * localStorage persistence for UI affordances ONLY. Conversation transcripts
 * live in the session-manager worker (see hooks/use-conversations.ts); the
 * legacy `iii-chat-conversations` blob is no longer read or written.
 */

const ACTIVE_KEY = 'iii-chat-active'
const LAST_MODEL_KEY = 'iii-chat-last-model'
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

const TURN_METRICS_KEY = 'iii-chat-turn-metrics'

/**
 * Whether the transcript shows a per-turn usage chip on each reply. Default
 * on — a chip nobody can find is a feature nobody uses — with an opt-out in
 * the session metrics dialog for readers who want a quiet transcript.
 */
export function loadShowTurnMetrics(): boolean {
  try {
    return localStorage.getItem(TURN_METRICS_KEY) !== 'off'
  } catch {
    return true
  }
}

export function saveShowTurnMetrics(show: boolean): void {
  try {
    localStorage.setItem(TURN_METRICS_KEY, show ? 'on' : 'off')
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

const RECENT_PROJECTS_KEY = 'iii-chat-recent-projects'
const RECENT_PROJECTS_MAX = 12

/**
 * Most-recently-used project directories, newest first. The directory picker
 * opens to this list so a returning user picks a known project in one click
 * instead of re-browsing the filesystem.
 */
export function loadRecentProjects(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_PROJECTS_KEY)
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

/** Promote `dir` to the front of the recent list (deduped, capped). */
export function saveRecentProject(dir: string): void {
  if (!dir) return
  try {
    const next = [dir, ...loadRecentProjects().filter((d) => d !== dir)].slice(
      0,
      RECENT_PROJECTS_MAX,
    )
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(next))
  } catch {
    /* best-effort */
  }
}

/** Forget a remembered project (the × affordance in the picker). */
export function removeRecentProject(dir: string): void {
  try {
    const next = loadRecentProjects().filter((d) => d !== dir)
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(next))
  } catch {
    /* best-effort */
  }
}
