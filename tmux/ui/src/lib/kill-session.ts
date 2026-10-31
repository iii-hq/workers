/**
 * What the Kill session confirm says. The console's confirm dialog is a
 * title, a short description and the actions (the ADE redesign, #1528,
 * dropped its `details` list), so the windows that die lead the
 * description: each one's label and what runs in it, capped the way the
 * kanban's delete dialog caps its keys.
 */

import type { AgentBadge, WindowRow } from './model'

/** Windows a confirm names before the rest fold into `N more`; one over the cap is named too. */
export const MAX_NAMED_WINDOWS = 6

/**
 * Characters one window may take in the sentence; a longer part ends in `…`.
 * With the cap on windows, it keeps the description to a few lines. The
 * console's dialog scrolls a longer text, but an older console clips it.
 */
export const MAX_PART_CHARS = 48

/** What a reader counts as characters: graphemes, so an emoji, a flag or an accent stays whole. */
function graphemes(text: string): string[] {
  if (typeof Intl === 'undefined' || !('Segmenter' in Intl)) return Array.from(text)
  const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' })
  return Array.from(segmenter.segment(text), (part) => part.segment)
}

/** `text` cut to `max` characters (graphemes), the last one `…`. */
export function clip(text: string, max = MAX_PART_CHARS): string {
  const chars = graphemes(text)
  return chars.length <= max ? text : `${chars.slice(0, Math.max(max - 1, 0)).join('').trimEnd()}…`
}

/** The parts of a window row the sentence reads. */
export type NamedWindow = Pick<WindowRow, 'label' | 'command' | 'compose'> & {
  window: Pick<WindowRow['window'], 'name'>
  agents: readonly Pick<AgentBadge, 'label'>[]
}

/**
 * `0:claude (Claude Code)`: the window and what runs in it — its agent, an
 * `iii compose` stack (whose process title is `iii:c:<namespace>`), or the
 * foreground command. Just `2:vim` when that is the window's own name. A
 * blank agent label falls through to the command.
 */
export function windowPart(row: NamedWindow): string {
  const running = row.agents[0]?.label.trim() || (row.compose ? 'iii compose' : row.command.trim())
  return running && running !== row.window.name ? `${row.label} (${running})` : row.label
}

/** `a, b and c`, each clipped, with the windows past the cap as `N more`. */
export function windowList(windows: readonly NamedWindow[]): string {
  const listed = windows.length > MAX_NAMED_WINDOWS + 1 ? MAX_NAMED_WINDOWS : windows.length
  const parts = [
    ...windows.slice(0, listed).map((row) => clip(windowPart(row))),
    ...(listed < windows.length ? [`${windows.length - listed} more`] : []),
  ]
  return parts.length < 2 ? (parts[0] ?? '') : `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`
}

/** The confirm's one sentence: which windows end, then who detaches. */
export function killSessionDescription(windows: readonly NamedWindow[]): string {
  const detaches = 'every terminal attached to the session — yours included — detaches.'
  if (windows.length === 0) return `Every window and process in it ends, and ${detaches}`
  const one = windows.length === 1
  return `${windowList(windows)} ${one ? 'ends' : 'end'} with every process in ${one ? 'it' : 'them'}, and ${detaches}`
}
