import type { SkillCatalogEntry, SkillCatalogUpdate } from '@/types/chat'

/**
 * The harness re-sends the model its skill index as a durable transcript
 * entry whenever the directory changes (`harness/src/skills.rs`). The model
 * needs the whole `<available_skills>` block; a reader only needs to know
 * that the index moved and, on request, what is in it now. This turns the
 * block back into rows so the timeline can show a one-line marker with the
 * list behind a disclosure instead of a wall of text.
 */

const BLOCK = /<available_skills>([\s\S]*?)<\/available_skills>/
const ROW = /^-\s+\*\*(.+?)\*\*\s*(?:[—–-]\s*(.*))?$/
const REMOVED = /^skill guidance is no longer available/i

function decodeEntities(value: string): string {
  return value
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
}

/**
 * `null` when the text is not a skill-index update at all; the caller keeps
 * its plain-notice fallback so an unfamiliar harness message still shows.
 */
export function parseSkillUpdate(text: string): SkillCatalogUpdate | null {
  const trimmed = text.trim()
  if (REMOVED.test(trimmed)) return { available: false, entries: [] }
  const block = BLOCK.exec(trimmed)
  if (!block) return null
  const entries: SkillCatalogEntry[] = []
  for (const line of block[1].split('\n')) {
    const row = ROW.exec(line.trim())
    if (!row) continue
    const id = decodeEntities(row[1]).trim()
    if (!id) continue
    entries.push({ id, description: decodeEntities(row[2] ?? '').trim() })
  }
  return { available: true, entries }
}

/** The collapsed row's summary, after the noun ("Skills …"). */
export function skillUpdateSummary(update: SkillCatalogUpdate): string {
  if (!update.available) return 'unavailable'
  const n = update.entries.length
  return n === 1 ? 'updated · 1 available' : `updated · ${n} available`
}
