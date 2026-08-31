/**
 * Pure state for the chat system-prompt and skill pickers. The selected values
 * are stored on the conversation record, and `ChatView` maps them onto the
 * first send. The harness inherits the prior turn's resolved prompt on later
 * sends that omit the prompt fields (see harness README § System prompt), so
 * the selection travels once and is frozen server-side for the whole session.
 *
 * The `custom` choice is vestigial: no surface renders that row any more —
 * authoring lives off-console, in `directory::system-prompts::create` /
 * `update` or the `system-prompts/*.md` files themselves — but
 * `toSelection` still handles it so a value persisted by an older build keeps
 * working.
 */

import type { SystemPromptSelection } from '@/lib/backend/harness-send'

export type PromptStrategy = 'enrich' | 'override'

/** `agent:` inside the named choice keeps agent selections distinct from fs
 * system prompts (prompt names are `[a-z0-9_-]`, so the prefix is
 * unambiguous). The id resolves server-side via `options.agent` (MOT-4485). */
export const AGENT_CHOICE_PREFIX = 'agent:'

/** Undefined/empty means every model-invocable skill; otherwise exact IDs. */
export type SkillSelection = string[] | undefined

/** What the select shows: provider default, a named fs prompt, or free text. */
export type SystemPromptChoice = 'default' | 'custom' | { named: string }

/** Legacy skill body kept only to decode established pre-ID-filter sessions. */
export interface SystemPromptAddon {
  kind: 'skill'
  /** Skill id (`directory::skills::*`). */
  name: string
  /** Body resolved at selection time, frozen server-side on the first send —
   * same contract as `namedBody`. */
  body: string
}

export interface SystemPromptState {
  choice: SystemPromptChoice
  strategy: PromptStrategy
  /** Body of the named prompt, resolved via directory::system-prompts::get at
   * selection time and frozen server-side on the first send — a file edit on
   * disk never reaches a session that has already started. */
  namedBody: string
  /** Custom textarea content; kept while switching choices. */
  customText: string
  /** Legacy skill bodies; new skill selections never write here. */
  addons: SystemPromptAddon[]
}

export const DEFAULT_SYSTEM_PROMPT_STATE: SystemPromptState = {
  choice: 'default',
  strategy: 'enrich',
  namedBody: '',
  customText: '',
  addons: [],
}

/** Agent id encoded in a new-session selection, or null for manual setup. */
export function agentIdFromSystemPrompt(
  state: SystemPromptState,
): string | null {
  if (
    typeof state.choice !== 'object' ||
    !state.choice.named.startsWith(AGENT_CHOICE_PREFIX)
  ) {
    return null
  }
  return state.choice.named.slice(AGENT_CHOICE_PREFIX.length) || null
}

/** Select an agent profile while preserving the dormant manual fields. */
export function withAgentChoice(
  state: SystemPromptState,
  agentId: string,
): SystemPromptState {
  return {
    ...state,
    choice: { named: `${AGENT_CHOICE_PREFIX}${agentId}` },
    namedBody: '',
    strategy: 'enrich',
  }
}

/** Enter manual setup without carrying an agent profile into the first send. */
export function withoutAgentChoice(
  state: SystemPromptState,
): SystemPromptState {
  return agentIdFromSystemPrompt(state) === null
    ? state
    : { ...state, choice: 'default', namedBody: '' }
}

/** Map picker state to the per-send selection; null = send no prompt fields. */
export function toSelection(
  s: SystemPromptState,
): SystemPromptSelection | null {
  const base =
    s.choice === 'default'
      ? ''
      : s.choice === 'custom'
        ? s.customText
        : s.namedBody
  const parts = [base, ...s.addons.map((a) => a.body)].filter((p) => p.trim())
  if (parts.length === 0) return null
  return {
    body: parts.join('\n\n'),
    /* Addons on top of a blank base (default choice, or a named/custom body
       that resolved empty) must never wipe the built-in prompt away, so a
       blank base always ships as enrich. */
    strategy: base.trim() ? s.strategy : 'enrich',
  }
}

/**
 * Selection for a given send. Only the session's first send carries the
 * prompt — once a turn has run (`turnEstablished`), the harness inherits the
 * prior turn's resolved prompt from sends that omit the fields. Re-sending
 * before a turn exists (e.g. retry after a failed first send) is harmless:
 * explicit fields always win with the same value.
 */
export function selectionForSend(
  s: SystemPromptState,
  turnEstablished: boolean,
): SystemPromptSelection | null {
  return turnEstablished ? null : toSelection(s)
}

/**
 * The agent id for a given send, or undefined. Gated harder than
 * `selectionForSend`: the harness REFUSES `options.agent` on a session with a
 * prior turn, and a queued mid-stream send targets exactly such a session —
 * so queue sends must never carry it (a re-sent prompt field is merely
 * re-resolved; a re-sent agent is an error).
 */
export function agentIdForSend(
  s: SystemPromptState,
  state: { turnEstablished: boolean; willQueue: boolean },
): string | undefined {
  if (state.turnEstablished || state.willQueue) return undefined
  return agentIdFromSystemPrompt(s) ?? undefined
}

export function toggleSkillSelection(
  current: SkillSelection,
  id: string,
): SkillSelection {
  if (!current?.length) return [id]
  const next = current.includes(id)
    ? current.filter((selected) => selected !== id)
    : [...current, id]
  return next.length > 0 ? next : undefined
}

export function skillSelectionForSend(
  current: SkillSelection,
  state: { turnEstablished: boolean; willQueue: boolean },
): SkillSelection {
  return state.turnEstablished || state.willQueue || !current?.length
    ? undefined
    : current
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
