/**
 * The context window as the breakdown draws it: one category list feeds both
 * the stacked bar (every entry is a segment) and the legend (the three
 * conversation entries grouped under a "Conversation" total row).
 */

import type { ContextSnapshot } from './metrics'

/**
 * Lightness only, no hue: the console's accent is neutral (it no longer
 * tells the conversation apart), and accent is rationed to live state
 * anyway. What the window is built from (prompt, skills, schemas, guidance,
 * overhead) sits dim; the conversation that grows turn by turn sits bright.
 * The dimmest step stays a clear notch above the empty track, and the bar
 * puts a 1px gap between segments, so neighbours never merge.
 */
const ink = (opacity: number) =>
  `color-mix(in srgb, var(--color-ink) ${opacity}%, transparent)`

/** The bar's empty track (styles.css `.harness-ui-stack`). */
export const COLOR_FREE = 'var(--color-surface-active)'

export type CategoryKey =
  | 'system'
  | 'skills'
  | 'tools'
  | 'user'
  | 'assistant'
  | 'results'
  | 'hooks'
  | 'overhead'

export interface Category {
  key: CategoryKey
  label: string
  color: string
  tokens: number
}

/** The three categories the legend groups under one "Conversation" row. */
const CONVERSATION_KEYS: CategoryKey[] = ['user', 'assistant', 'results']

/** Every category of the assembled window, in bar order. */
export function categories(snapshot: ContextSnapshot): Category[] {
  const cats = snapshot.categories
  const messages = cats.messages
  return [
    { key: 'system', label: 'System prompt', color: ink(46), tokens: cats.system_prompt },
    { key: 'skills', label: 'Skills', color: ink(38), tokens: cats.skills ?? 0 },
    { key: 'tools', label: 'Function schemas', color: ink(32), tokens: cats.tools },
    { key: 'user', label: 'Your messages', color: ink(95), tokens: messages.user },
    { key: 'assistant', label: 'Assistant', color: ink(76), tokens: messages.assistant },
    {
      key: 'results',
      label: 'Function results',
      color: ink(60),
      tokens: messages.function_result + messages.custom,
    },
    {
      key: 'hooks',
      label: 'Hook guidance',
      color: ink(28),
      // Optional on the wire (serde default): absent in snapshots written
      // before the category existed.
      tokens: cats.hook_guidance ?? 0,
    },
    { key: 'overhead', label: 'Overhead', color: ink(24), tokens: cats.overhead },
  ]
}

export interface LegendEntry {
  key: string
  label: string
  /** `null` renders an invisible swatch — the row is not a bar segment. */
  color: string | null
  tokens: number
  badge?: string
  /** The conversation total: its parts follow as `sub` rows. */
  group?: boolean
  sub?: boolean
}

/**
 * The legend, derived from the same category array the bar draws. Empty rows
 * are dropped; the conversation entries follow a "Conversation" total row as
 * indented parts; and the two rows that are not bar segments (the compaction
 * summary, free space) take their place around the overhead row.
 */
export function legendRows(snapshot: ContextSnapshot, cats: Category[]): LegendEntry[] {
  const rows: LegendEntry[] = []
  for (const category of cats) {
    if (category.key === 'user') {
      const parts = cats.filter((c) => CONVERSATION_KEYS.includes(c.key) && c.tokens > 0)
      const tokens = parts.reduce((sum, entry) => sum + entry.tokens, 0)
      if (tokens > 0) {
        rows.push({ key: 'conversation', label: 'Conversation', color: ink(76), tokens, group: true })
        for (const part of parts) rows.push({ ...part, sub: true })
      }
      continue
    }
    if (CONVERSATION_KEYS.includes(category.key)) continue
    if (category.key === 'overhead' && snapshot.compacted) {
      rows.push({
        key: 'summary',
        label: 'Summary',
        color: null,
        tokens: snapshot.summarized_head_tokens ?? 0,
        badge: 'compacted',
      })
    }
    if (category.tokens > 0) rows.push(category)
  }
  rows.push({ key: 'free', label: 'Free', color: COLOR_FREE, tokens: snapshot.free })
  return rows
}

/** A share of the window; a sliver that rounds to 0 reads `<1%`, not `0%`. */
export function percentOf(tokens: number, usable: number): string {
  if (usable <= 0 || tokens <= 0) return '0%'
  const pct = Math.round((tokens / usable) * 100)
  return pct === 0 ? '<1%' : `${pct}%`
}
