/**
 * Reading back the notes the judge's mention hook leaves for the model
 * (`judge/src/mentions/render.rs`): the `<mention_providers>` index, which
 * is agent bookkeeping, and the `<mentions>` block, which says what the
 * agent was told about each mentioned item. The chat shows the latter as a
 * quiet row instead of raw tags.
 */

import { findMentions } from './token'

export const MENTIONS_NOTE_TAG = '<mentions>'
export const PROVIDERS_NOTE_TAG = '<mention_providers>'

export type MentionNoteStatus =
  | 'resolved'
  | 'not-found'
  | 'unknown-provider'
  | 'error'

export interface MentionNoteEntry {
  name: string
  id: string
  status: MentionNoteStatus
  /** The provider's one-line summary (resolved) or why it did not resolve. */
  summary: string
  /** The pre-verified details call, as written (`fn {payload}`). */
  details?: string
}

/** The index of mention names an agent may write — never shown in chat. */
export function isProvidersNote(text: string): boolean {
  return text.trimStart().startsWith(PROVIDERS_NOTE_TAG)
}

function statusOf(summary: string): MentionNoteStatus {
  if (summary.startsWith('not found')) return 'not-found'
  if (summary.startsWith('no installed worker provides'))
    return 'unknown-provider'
  if (summary.startsWith('could not be resolved')) return 'error'
  return 'resolved'
}

/**
 * The entries of a `<mentions>` note, or `null` when the text is not one.
 * Entry lines read `- @<name>(id="…") — <summary>`, optionally followed by
 * `  details: <function> <payload>`.
 */
export function parseMentionsNote(text: string): MentionNoteEntry[] | null {
  if (!text.trimStart().startsWith(MENTIONS_NOTE_TAG)) return null
  const entries: MentionNoteEntry[] = []
  for (const line of text.split('\n')) {
    const details = line.match(/^\s+details:\s+(.+)$/)
    if (details) {
      const last = entries.at(-1)
      if (last) last.details = details[1].trim()
      continue
    }
    if (!line.startsWith('- @')) continue
    const [token] = findMentions(line)
    if (token?.index !== 2) continue
    const rest = line.slice(2 + token.token.length)
    const summary = rest.replace(/^\s*—\s*/, '').trim()
    entries.push({
      name: token.name,
      id: token.id,
      status: statusOf(summary),
      summary,
    })
  }
  return entries
}
