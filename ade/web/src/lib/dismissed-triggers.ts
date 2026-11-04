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

export function saveDismissedTriggers(ids: ReadonlySet<string>): void {
  try {
    globalThis.localStorage?.setItem(
      STORAGE_KEY,
      JSON.stringify([...ids].slice(-MAX_DISMISSED)),
    )
  } catch {
    // best-effort
  }
}
