/**
 * Pure state for the chat system-prompt picker. `SystemPromptPicker.tsx`
 * renders it on the new-session screen; the chosen value is stored on
 * `Conversation.systemPrompt` (see `use-conversations`), and `ChatView` maps
 * it onto `ChatStreamOptions.systemPrompt` via `toSelection` at each send.
 *
 * The `custom` choice is vestigial: no surface renders that row any more —
 * authoring lives in the iii-directory UI's system-prompts tab — but
 * `toSelection` still handles it so a value persisted by an older build keeps
 * working.
 */

import type { SystemPromptSelection } from '@/lib/backend/harness-send'

export type PromptStrategy = 'enrich' | 'override'

/** What the select shows: provider default, a named fs prompt, or free text. */
export type SystemPromptChoice = 'default' | 'custom' | { named: string }

export interface SystemPromptState {
  choice: SystemPromptChoice
  strategy: PromptStrategy
  /** Body of the named prompt, resolved via directory::system-prompts::get at
   * selection time. Stale if the file changes on disk mid-session —
   * reselect to refresh. */
  namedBody: string
  /** Custom textarea content; kept while switching choices. */
  customText: string
}

export const DEFAULT_SYSTEM_PROMPT_STATE: SystemPromptState = {
  choice: 'default',
  strategy: 'enrich',
  namedBody: '',
  customText: '',
}

/** Map picker state to the per-send selection; null = send no prompt fields. */
export function toSelection(
  s: SystemPromptState,
): SystemPromptSelection | null {
  if (s.choice === 'default') return null
  const body = s.choice === 'custom' ? s.customText : s.namedBody
  if (!body.trim()) return null
  return { body, strategy: s.strategy }
}

/** Radix Select needs string values; `named:` prefixes fs prompt names
 * (names are `[a-z0-9_-]`, so the prefix is unambiguous). */
export function choiceToValue(c: SystemPromptChoice): string {
  if (c === 'default' || c === 'custom') return c
  return `named:${c.named}`
}

export function valueToChoice(v: string): SystemPromptChoice {
  if (v === 'default' || v === 'custom') return v
  return { named: v.slice('named:'.length) }
}
