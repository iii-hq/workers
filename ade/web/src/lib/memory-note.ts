/**
 * The memory worker's `<memory …>` notes, read back into what they say. The
 * worker writes three shapes (memory/src/hooks.rs):
 *
 * - a recall: `<memory bank="b">`, a header naming the day it ran
 *   (`… recalled 2026-11-23 Monday; …`), then `- text (YYYY-MM-DD)` lines;
 * - the rules reminder, alone or after a recall's lines: a line opening with
 *   "Standing instructions — apply every one to this reply:" and the rules;
 * - a rules update: `<memory update="rules">`, a sentence, the new section.
 *
 * Anything else is not a memory note (`null`): the row shows the raw text.
 */

export interface RecalledMemory {
  text: string
  /** When the fact was last stated, `YYYY-MM-DD`. */
  date: string | null
}

export interface MemoryNote {
  /** The bank recalled from; `null` on a rules update. */
  bank: string | null
  /** The day the recall ran, as written (`2026-11-23 Monday`). */
  recalled: string | null
  memories: RecalledMemory[]
  /** The rules repeated next to the reply, as written. */
  reminder: string | null
  /** A rules update: the section that replaced the system prompt's. */
  rulesUpdate: string | null
}

const REMINDER_OPEN = 'Standing instructions — apply every one to this reply:'
const RULES_UPDATE_LEAD = /^The memory rules changed\.[^:]*:\s*/
const OPEN = /^<memory\s+(bank|update)="([^"]*)">\n?/
const CLOSE = /\n?<\/memory>\s*$/
const LINE = /^- (.*?)(?: \((\d{4}-\d{2}-\d{2})\))?$/

export function parseMemoryNote(text: string): MemoryNote | null {
  const trimmed = text.trim()
  const open = trimmed.match(OPEN)
  if (!open || !CLOSE.test(trimmed)) return null
  const body = trimmed.slice(open[0].length).replace(CLOSE, '')
  if (open[1] === 'update') {
    if (open[2] !== 'rules') return null
    return {
      bank: null,
      recalled: null,
      memories: [],
      reminder: null,
      rulesUpdate: body.replace(RULES_UPDATE_LEAD, '').trim(),
    }
  }

  const note: MemoryNote = {
    bank: open[2] || null,
    recalled: null,
    memories: [],
    reminder: null,
    rulesUpdate: null,
  }
  const lines = body.split('\n')
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    // Rule bodies may hold `- ` lines of their own: everything from the
    // reminder on is the reminder.
    if (line.startsWith(REMINDER_OPEN)) {
      const rest = [line.slice(REMINDER_OPEN.length), ...lines.slice(i + 1)]
      note.reminder = rest.join('\n').trim()
      break
    }
    const recalled = line.match(
      /^Relevant remembered memories \(.*?recalled ([^;)]+)/,
    )
    if (recalled) {
      note.recalled = recalled[1].trim()
      continue
    }
    const item = line.match(LINE)
    if (item) {
      note.memories.push({ text: item[1], date: item[2] ?? null })
      continue
    }
    if (line.trim()) return null
  }
  if (note.memories.length === 0 && note.reminder === null) return null
  return note
}
