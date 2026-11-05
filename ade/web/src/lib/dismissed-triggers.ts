/**
 * Inactive trigger rows the person dismissed from a chat's trigger card.
 * The rows are rebuilt from the transcript on every load, so a dismissal
 * kept only in component state came back on reload; the subscription ids
 * are kept here (localStorage) instead. Ids are unique across sessions, so
 * one list serves every chat; the oldest leave first past the cap.
 */

const STORAGE_KEY = 'console:chat:dismissed-triggers'
const MAX_DISMISSED = 1000

export function loadDismissedTriggers(): Set<string> {
  try {
    const raw = JSON.parse(
      globalThis.localStorage?.getItem(STORAGE_KEY) ?? 'null',
    )
    return new Set(
      Array.isArray(raw)
        ? raw.filter((id): id is string => typeof id === 'string')
        : [],
    )
  } catch {
    return new Set()
  }
}

const CHANGE_EVENT = 'iii:dismissed-triggers'

/**
 * Save, merged with what is stored: two panes on the same chat each hold
 * their own copy, and a save from one must not drop the other's dismissals.
 * Mounted cards hear the change through `onDismissedTriggersChange`.
 */
export function saveDismissedTriggers(ids: ReadonlySet<string>): Set<string> {
  const merged = new Set([...loadDismissedTriggers(), ...ids])
  try {
    globalThis.localStorage?.setItem(
      STORAGE_KEY,
      JSON.stringify([...merged].slice(-MAX_DISMISSED)),
    )
  } catch {
    // best-effort
  }
  globalThis.dispatchEvent?.(new Event(CHANGE_EVENT))
  return merged
}

/** Follow dismissals saved by another card (or another browser tab). */
export function onDismissedTriggersChange(listener: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === STORAGE_KEY) listener()
  }
  globalThis.addEventListener?.(CHANGE_EVENT, listener)
  globalThis.addEventListener?.('storage', onStorage)
  return () => {
    globalThis.removeEventListener?.(CHANGE_EVENT, listener)
    globalThis.removeEventListener?.('storage', onStorage)
  }
}
