/**
 * localStorage persistence for UI affordances ONLY. Conversation transcripts
 * live in the session-manager worker (see hooks/use-conversations.ts); the
 * legacy `iii-chat-conversations` blob is no longer read or written.
 */

const ACTIVE_KEY = 'iii-chat-active'
const LAST_MODEL_KEY = 'iii-chat-last-model'
const LAST_THINKING_LEVEL_KEY = 'iii-chat-last-thinking-level'
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

export function loadLastThinkingLevel(): string | null {
  try {
    const level = localStorage.getItem(LAST_THINKING_LEVEL_KEY)
    return level && level.length > 0 ? level : null
  } catch {
    return null
  }
}

export function saveLastThinkingLevel(level: string): void {
  try {
    localStorage.setItem(LAST_THINKING_LEVEL_KEY, level)
  } catch {
    /* best-effort */
  }
}

const EDGE_ADD_DISCOVERED_KEY = 'iii-edge-add-discovered'

/**
 * Whether the user has ever added a panel through an edge add zone (either
 * side) — gates the first-run affordance: the framed `+` sliver on each
 * edge, its periodic shake and the preview's hint. After discovery the
 * edges are bare and only the split preview answers a dwell or a tap.
 * Existing splits don't count: the default workspace ships with a
 * 2-column tab.
 */
export function loadEdgeAddDiscovered(): boolean {
  try {
    return localStorage.getItem(EDGE_ADD_DISCOVERED_KEY) === '1'
  } catch {
    // Storage unavailable means the flag could never persist — stay quiet
    // rather than nudge on every visit.
    return true
  }
}

export function saveEdgeAddDiscovered(): void {
  try {
    localStorage.setItem(EDGE_ADD_DISCOVERED_KEY, '1')
  } catch {
    /* best-effort */
  }
}
