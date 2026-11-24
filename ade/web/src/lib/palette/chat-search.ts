/**
 * Chats found by what was said in them: `session::search` hits as palette
 * rows.
 *
 * The server matches fuzzily (word starts, typos) over titles and message
 * text, and says where: ranges in the title, and a snippet of the best
 * message. Rows keep the local chat row's id (`chat:<id>`), so a hit replaces
 * the title-only row and lands in recents like any chat.
 */

import { formatRelative } from '@/lib/relative-age'
import type { SessionSearchHit, TextRange } from '@/lib/sessions/api'
import type { MatchRange, PaletteEntry } from './sources'

/** Characters of context kept before a snippet's first match: enough to
 *  read the match in its sentence, few enough that it is on screen. */
const SNIPPET_LEAD = 24

/**
 * Cut a snippet so its first match sits near the start: the row truncates
 * at its right edge, and a match past it would hide why the chat was found.
 */
export function focusSnippet(
  text: string,
  ranges: readonly TextRange[],
  lead = SNIPPET_LEAD,
): { text: string; ranges: MatchRange[] } {
  const first = ranges[0]?.[0] ?? 0
  if (first <= lead + 1) {
    return { text, ranges: ranges.map(([a, b]) => [a, b] as const) }
  }
  // Start at a word boundary inside the lead, so no word is cut in half.
  let start = first - lead
  const space = text.indexOf(' ', start)
  if (space !== -1 && space < first) start = space + 1
  const shift = start - 1 // the `…` takes one unit
  return {
    text: `…${text.slice(start)}`,
    ranges: ranges.map(([a, b]) => [a - shift, b - shift] as const),
  }
}

export interface SearchableChat {
  id: string
  title: string
  /** Milliseconds since epoch; the row's age tag. */
  updatedAt: number
}

/**
 * Rows for the hits that name a chat the sidebar shows, in the server's
 * order. A hit outside that list (a side chat, a sub-agent, an old chat the
 * sidebar never paged in) has nowhere to open, so it is left out.
 */
export function chatSearchEntries(
  hits: readonly SessionSearchHit[],
  chats: ReadonlyMap<string, SearchableChat>,
  open: (id: string) => void,
): PaletteEntry[] {
  const rows: PaletteEntry[] = []
  for (const hit of hits) {
    const chat = chats.get(hit.session_id)
    if (!chat) continue
    const title = chat.title || 'untitled chat'
    rows.push({
      id: `chat:${chat.id}`,
      kind: 'chat',
      title,
      // Two chats can share a title; their ages tell them apart.
      meta: formatRelative(chat.updatedAt),
      // The sidebar may show a title the server has not stored yet; its
      // ranges only fit the title they were measured on.
      titleRanges: title === hit.title ? hit.title_ranges : undefined,
      snippet: hit.snippet
        ? focusSnippet(hit.snippet.text, hit.snippet.ranges)
        : undefined,
      run: () => open(chat.id),
    })
  }
  return rows
}
