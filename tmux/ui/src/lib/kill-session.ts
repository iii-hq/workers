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

/** Of a part's characters, at most this many go to what runs in the window, so a long window name never hides it. */
export const MAX_RUNNING_CHARS = 17

/**
 * `0:claude (Claude Code)`: the window and what runs in it — its first
 * agent with a label, an `iii compose` stack (whose process title is
 * `iii:c:<namespace>`), or the foreground command. Just `2:vim` when that is
 * the window's own name. At most `MAX_PART_CHARS`: the window and what runs
 * in it are clipped apart, so neither pushes the other out.
 */
export function windowPart(row: NamedWindow): string {
  const agent = row.agents.map((badge) => badge.label.trim()).find((label) => label !== '')
  const running = agent || (row.compose ? 'iii compose' : row.command.trim())
  if (!running || running === row.window.name) return clip(row.label)
  const shown = clip(running, MAX_RUNNING_CHARS)
  // ` (` and `)` take the other three.
  return `${clip(row.label, MAX_PART_CHARS - graphemes(shown).length - 3)} (${shown})`
}

/**
 * `text` cut in the middle to `max` characters (graphemes), keeping both
 * ends: names that share a long beginning (`acme-api-staging-…`) usually
 * differ at the end.
 */
export function clipMiddle(text: string, max = MAX_PART_CHARS): string {
  const chars = graphemes(text)
  if (chars.length <= max) return text
  const head = Math.ceil((max - 1) / 2)
  const tail = max - 1 - head
  return `${chars.slice(0, head).join('').trimEnd()}…${chars.slice(chars.length - tail).join('').trimStart()}`
}

/** `a, b and c`, with the windows past the cap as `N more`. */
export function windowList(windows: readonly NamedWindow[]): string {
  const listed = windows.length > MAX_NAMED_WINDOWS + 1 ? MAX_NAMED_WINDOWS : windows.length
  const parts = [
    ...windows.slice(0, listed).map(windowPart),
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
