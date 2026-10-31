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
