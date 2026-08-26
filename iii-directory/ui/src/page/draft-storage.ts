/**
 * Unsaved-work persistence for the directory editor.
 *
 * The console unmounts the directory page on every tab switch, so without
 * this a mid-typed new entry — or an unsaved edit to an existing one — dies
 * with the component. These are the pure decisions; `browser.tsx` owns the
 * localStorage calls.
 */

export interface StoredDraft {
  /** The draft is a not-yet-created entry (no on-disk key yet). */
  creating: boolean
  /** Entry the draft edits; `null` while creating. */
  key: string | null
  content: string
}

export type DraftAction =
  | { kind: 'write'; draft: StoredDraft }
  | { kind: 'clear' }
  /** Leave storage untouched — see the in-flight case in [`draftAction`]. */
  | { kind: 'keep' }

/** Tolerant read: anything not shaped like a draft is treated as absent, so
 * a hand-edited or stale storage entry can't wedge the editor on mount. */
export function parseStoredDraft(raw: string | null): StoredDraft | null {
  if (!raw) return null
  try {
    const parsed = JSON.parse(raw)
    if (typeof parsed?.content !== 'string') return null
    return {
      creating: parsed.creating === true,
      key: typeof parsed.key === 'string' ? parsed.key : null,
      content: parsed.content,
    }
  } catch {
    return null
  }
}

/**
 * What storage should hold for the editor's current state.
 *
 * `loadedContent` is the on-disk baseline the draft is diffed against, and
 * `null` means there is no baseline yet — a load is in flight, or a restored
 * draft is still waiting for one. That case must `keep`: clearing it would
 * destroy restored work in the window before its baseline lands.
 */
export function draftAction({
  creating,
  selected,
  draft,
  loadedContent,
}: {
  creating: boolean
  selected: string | null
  draft: string
  loadedContent: string | null
}): DraftAction {
  if (loadedContent === null) {
    return creating || selected ? { kind: 'keep' } : { kind: 'clear' }
  }
  if (creating) {
    return { kind: 'write', draft: { creating: true, key: null, content: draft } }
  }
  if (selected && draft !== loadedContent) {
    return {
      kind: 'write',
      draft: { creating: false, key: selected, content: draft },
    }
  }
  // Clean, or nothing open: no unsaved work to keep.
  return { kind: 'clear' }
}
