/* The IDE editor's display preferences: code size, and whether the header
   shows the file's size and line count and a Save button. They belong to
   the person reading the screen, not to the machine running the worker, so
   they live in localStorage (as the terminal font size does) rather than in
   the `ide` configuration entry. Every editor in every tab follows one
   value: a `storage` event carries a change between tabs, a CustomEvent
   within one. */

import { useCallback, useSyncExternalStore } from 'react'

export const MIN_EDITOR_FONT_SIZE = 10
export const MAX_EDITOR_FONT_SIZE = 24
export const DEFAULT_EDITOR_FONT_SIZE = 13

/** The code faces on offer. Geist Mono ships with the console; the others
    render when installed on the machine and fall back to it otherwise. */
export const EDITOR_FONT_FAMILIES: readonly { id: string; label: string; stack: string }[] = [
  { id: 'geist-mono', label: 'Geist Mono (default)', stack: '"Geist Mono"' },
  { id: 'jetbrains-mono', label: 'JetBrains Mono', stack: '"JetBrains Mono", "JetBrainsMono Nerd Font"' },
  { id: 'fira-code', label: 'Fira Code', stack: '"Fira Code", "FiraCode Nerd Font"' },
  { id: 'cascadia-code', label: 'Cascadia Code', stack: '"Cascadia Code", "Cascadia Mono"' },
  { id: 'source-code-pro', label: 'Source Code Pro', stack: '"Source Code Pro"' },
  { id: 'ibm-plex-mono', label: 'IBM Plex Mono', stack: '"IBM Plex Mono"' },
  { id: 'sf-mono', label: 'SF Mono', stack: '"SF Mono", ui-monospace' },
  { id: 'menlo', label: 'Menlo', stack: 'Menlo' },
  { id: 'consolas', label: 'Consolas', stack: 'Consolas' },
  { id: 'system', label: 'System monospace', stack: 'ui-monospace, SFMono-Regular, Menlo, Consolas' },
]
export const DEFAULT_EDITOR_FONT_FAMILY = 'geist-mono'
const FALLBACK_STACK = '"Geist Mono", ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace'

export function isEditorFontFamily(value: unknown): value is string {
  return typeof value === 'string' && EDITOR_FONT_FAMILIES.some((family) => family.id === value)
}

/** The CSS font-family value for a family id, always ending in the console's own stack. */
export function editorFontStack(id: string): string {
  const family = EDITOR_FONT_FAMILIES.find((candidate) => candidate.id === id)
  if (!family || family.id === DEFAULT_EDITOR_FONT_FAMILY) return FALLBACK_STACK
  return `${family.stack}, ${FALLBACK_STACK}`
}

export interface EditorPrefs {
  fontSize: number
  /** One of EDITOR_FONT_FAMILIES' ids: the face for code, diffs and the terminal. */
  fontFamily: string
  /** Size and line count beside the breadcrumb (the count opens Go to line). */
  showFileInfo: boolean
  /** A Save button in the header; ⌘S / Ctrl+S saves either way. */
  showSaveButton: boolean
}

export const DEFAULT_EDITOR_PREFS: EditorPrefs = {
  fontSize: DEFAULT_EDITOR_FONT_SIZE,
  fontFamily: DEFAULT_EDITOR_FONT_FAMILY,
  showFileInfo: false,
  showSaveButton: false,
}

const STORAGE_KEY = 'iii::ide::editor-prefs'
const CHANGE_EVENT = 'iii:ide-editor-prefs'

export function clampEditorFontSize(value: unknown): number {
  const size = Number(value)
  if (!Number.isFinite(size)) return DEFAULT_EDITOR_FONT_SIZE
  return Math.min(MAX_EDITOR_FONT_SIZE, Math.max(MIN_EDITOR_FONT_SIZE, Math.round(size)))
}

/** Anything unreadable falls back field by field, never as a whole. */
export function parseEditorPrefs(raw: string | null | undefined): EditorPrefs {
  if (!raw) return DEFAULT_EDITOR_PREFS
  try {
    const value = JSON.parse(raw) as Partial<Record<keyof EditorPrefs, unknown>> | null
    if (value === null || typeof value !== 'object') return DEFAULT_EDITOR_PREFS
    return {
      fontSize: value.fontSize === undefined ? DEFAULT_EDITOR_FONT_SIZE : clampEditorFontSize(value.fontSize),
      fontFamily: isEditorFontFamily(value.fontFamily) ? value.fontFamily : DEFAULT_EDITOR_FONT_FAMILY,
      showFileInfo: typeof value.showFileInfo === 'boolean' ? value.showFileInfo : DEFAULT_EDITOR_PREFS.showFileInfo,
      showSaveButton:
        typeof value.showSaveButton === 'boolean' ? value.showSaveButton : DEFAULT_EDITOR_PREFS.showSaveButton,
    }
  } catch {
    return DEFAULT_EDITOR_PREFS
  }
}

/** Storage is absent off the browser and blocked in some privacy modes. */
function storage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : (window.localStorage ?? null)
  } catch {
    return null
  }
}

// useSyncExternalStore wants a stable snapshot: re-read only when the raw
// string changes.
let cached: { raw: string | null; prefs: EditorPrefs } | null = null

export function readEditorPrefs(): EditorPrefs {
  const store = storage()
  // Blocked storage: whatever this page set stands until a reload.
  if (store === null) return cached?.prefs ?? DEFAULT_EDITOR_PREFS
  const raw = store.getItem(STORAGE_KEY)
  if (cached === null || cached.raw !== raw) cached = { raw, prefs: parseEditorPrefs(raw) }
  return cached.prefs
}

export function writeEditorPrefs(patch: Partial<EditorPrefs>): EditorPrefs {
  const next = parseEditorPrefs(JSON.stringify({ ...readEditorPrefs(), ...patch }))
  const raw = JSON.stringify(next)
  try {
    storage()?.setItem(STORAGE_KEY, raw)
  } catch {
    // Full or blocked: the change still applies until a reload.
  }
  cached = { raw: storage()?.getItem(STORAGE_KEY) ?? null, prefs: next }
  if (typeof window !== 'undefined') window.dispatchEvent(new CustomEvent(CHANGE_EVENT))
  return next
}

function subscribe(onChange: () => void): () => void {
  if (typeof window === 'undefined') return () => {}
  const onStorage = (event: StorageEvent) => {
    if (event.key === null || event.key === STORAGE_KEY) onChange()
  }
  window.addEventListener('storage', onStorage)
  window.addEventListener(CHANGE_EVENT, onChange)
  return () => {
    window.removeEventListener('storage', onStorage)
    window.removeEventListener(CHANGE_EVENT, onChange)
  }
}

export function useEditorPrefs(): [EditorPrefs, (patch: Partial<EditorPrefs>) => void] {
  const prefs = useSyncExternalStore(subscribe, readEditorPrefs, () => DEFAULT_EDITOR_PREFS)
  const update = useCallback((patch: Partial<EditorPrefs>) => void writeEditorPrefs(patch), [])
  return [prefs, update]
}
