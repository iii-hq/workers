import { useSyncExternalStore } from 'react'

const KEY = (checkId: string) => `iii-cloud:asked:${checkId}`
const listeners = new Set<() => void>()
const memory = new Map<string, string>()

/** When the person handed this check to the agent: remembered per check id, best effort. */
export function askedAt(checkId: string): string | null {
  try {
    return localStorage.getItem(KEY(checkId)) ?? memory.get(checkId) ?? null
  } catch {
    return memory.get(checkId) ?? null
  }
}

export function markAsked(checkId: string, at = new Date().toISOString()) {
  memory.set(checkId, at)
  try {
    localStorage.setItem(KEY(checkId), at)
  } catch {
    // storage is best effort: the in-memory copy keeps this tab right
  }
  for (const l of [...listeners]) l()
}

export function useAsked(checkId: string | undefined): string | null {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
    () => (checkId ? askedAt(checkId) : null),
  )
}

/** 14:34 in the person's time zone; English (en-GB reads 24 h) whatever the browser's language. */
export const clock = (iso: string) =>
  new Date(iso).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', hour12: false })

/** Forget every remembered hand-off (this tab's memory and the stored keys). */
export function clearAsked() {
  memory.clear()
  folderNotes.clear()
  launch = null
  try {
    for (const k of Object.keys(localStorage)) if (k.startsWith('iii-cloud:')) localStorage.removeItem(k)
  } catch {
    // no storage: memory was all there was
  }
  for (const l of [...listeners]) l()
}

const folderNotes = new Set<string>()

/** The hand-off could not move the chat into the agent's folder: the person confirms the folder in the chat. */
export function markFolderNote(checkId: string) {
  folderNotes.add(checkId)
  for (const l of [...listeners]) l()
}

export function useFolderNote(checkId: string | undefined): boolean {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
    () => (checkId ? folderNotes.has(checkId) : false),
  )
}

export const FOLDER_NOTE = 'Confirm the folder in the chat when asked.'

/** How the last hand-off reached a chat: delivered to the agent's own, started in a new one, or not at all. */
export type Launch =
  | { kind: 'launching' }
  | { kind: 'sent' }
  | { kind: 'waiting' }
  | { kind: 'started' }
  | { kind: 'error'; retry: () => void }
  | null
let launch: Launch = null

export function setLaunch(next: Launch) {
  launch = next
  for (const l of [...listeners]) l()
}

export const isLaunching = () => launch?.kind === 'launching'

export function useLaunch(): Launch {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
    () => launch,
  )
}
