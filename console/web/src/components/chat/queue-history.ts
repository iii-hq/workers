import type { Attachment } from '@/types/chat'

/** One browsable queued message (this tab's drafts, oldest→newest). */
export interface QueuedForEdit {
  id: string
  text: string
  attachments: Attachment[]
}

/** What the composer should load into the editor, or `null` = null-id live draft. */
export interface HistoryTarget {
  id: string | null
  text: string
  attachments: Attachment[]
}

export type HistoryNavResult =
  /** Let the arrow do its normal caret move. */
  | { kind: 'noop' }
  /** Replace the editor with this message ('' text = back to a live draft). */
  | { kind: 'load'; target: HistoryTarget }

/**
 * Pure ↑/↓ queue-history navigation. `browseId` is the message the editor
 * currently holds (`null` = a live draft); `current` is the editor's text.
 *
 * Navigation only happens while the editor is *pristine* — blank for a live
 * draft, or exactly the browsed message unedited — so an in-progress edit and
 * caret moves within it are never clobbered. ↑ goes older (from a blank
 * composer, enters at the newest); ↓ goes newer (past the newest, exits to a
 * blank draft). Ends of the range that can't advance are `noop`.
 */
export function nextHistoryTarget(
  list: QueuedForEdit[],
  browseId: string | null,
  current: string,
  direction: 'up' | 'down',
): HistoryNavResult {
  const original = browseId
    ? (list.find((m) => m.id === browseId)?.text ?? '')
    : ''
  const pristine = browseId === null ? current === '' : current === original
  if (!pristine) return { kind: 'noop' }

  const load = (target: HistoryTarget): HistoryNavResult => ({
    kind: 'load',
    target,
  })
  const idx = browseId ? list.findIndex((m) => m.id === browseId) : -1

  if (direction === 'up') {
    if (browseId === null) {
      const newest = list[list.length - 1]
      return newest ? load(newest) : { kind: 'noop' }
    }
    const older = idx > 0 ? list[idx - 1] : null
    return older ? load(older) : { kind: 'noop' }
  }

  // down
  if (browseId === null) return { kind: 'noop' }
  const newer = idx >= 0 && idx < list.length - 1 ? list[idx + 1] : null
  if (newer) return load(newer)
  return load({ id: null, text: '', attachments: [] })
}
