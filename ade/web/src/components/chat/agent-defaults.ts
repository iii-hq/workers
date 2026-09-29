/**
 * First-run agent selection for the new-session screen, kept pure so the
 * rules are testable without rendering:
 *
 *   - which Directory profile the gallery selects on its own (`default`,
 *     shown as "Default"), so the card the UI marks is the profile the first
 *     send actually runs — an unselected send would otherwise fall back to
 *     the harness's embedded identity, which is a different profile;
 *   - how a catalog row becomes the snapshot frozen onto the session;
 *   - the profile's optional composer example (`composer_placeholder`),
 *     a hint inside an EMPTY composer — never a draft, never sent.
 */

import type { AgentEntry } from '@/lib/backend/directory-prompts'
import type {
  AgentProfileSnapshot,
  SubagentColor,
  SubagentIcon,
} from '@/types/chat'
import type { SystemPromptState } from './system-prompt-selection'

/** The bundled Directory profile a new session runs when nothing else is chosen. */
export const DEFAULT_AGENT_ID = 'default'

/** Mirrors iii-directory's `AGENT_COMPOSER_PLACEHOLDER_MAX_CHARS`. */
export const COMPOSER_PLACEHOLDER_MAX_CHARS = 200

/** The composer's hint when the selected profile has no example of its own. */
export const GENERIC_COMPOSER_PLACEHOLDER = 'send a message…'

/**
 * Plain-text example from a catalog row or persisted metadata: whitespace
 * collapsed, blank or non-string = none, capped defensively at the server
 * limit (React renders it as text, never HTML).
 */
export function normalizeComposerPlaceholder(
  value: unknown,
): string | undefined {
  if (typeof value !== 'string') return undefined
  const collapsed = value.split(/\s+/).filter(Boolean).join(' ')
  if (!collapsed) return undefined
  return Array.from(collapsed).slice(0, COMPOSER_PLACEHOLDER_MAX_CHARS).join('')
}

/** The snapshot a gallery card freezes onto the session when selected. */
export function agentProfileFromEntry(entry: AgentEntry): AgentProfileSnapshot {
  const composerPlaceholder = normalizeComposerPlaceholder(
    entry.composer_placeholder,
  )
  return {
    id: entry.id,
    name: entry.name.trim() || entry.id,
    ...(entry.model ? { model: entry.model } : {}),
    ...(entry.reasoning_effort
      ? { reasoningEffort: entry.reasoning_effort }
      : {}),
    ...(entry.icon ? { icon: entry.icon as SubagentIcon } : {}),
    ...(entry.color ? { color: entry.color as SubagentColor } : {}),
    ...(composerPlaceholder ? { composerPlaceholder } : {}),
  }
}

/**
 * The gallery row to select automatically, or null. Only when the session
 * has no agent yet AND is still on the untouched default prompt (a named or
 * custom system prompt is a deliberate manual setup), and only when the
 * Directory actually serves a visible `default` row — an older Directory
 * without it keeps the previous no-selection behavior.
 */
export function agentToPreselect(
  visibleEntries: readonly AgentEntry[],
  selectedAgentId: string | null,
  systemPrompt: SystemPromptState,
): AgentEntry | null {
  if (selectedAgentId !== null) return null
  if (systemPrompt.choice !== 'default') return null
  return (
    visibleEntries.find(
      (entry) => entry.id === DEFAULT_AGENT_ID && !entry.hidden,
    ) ?? null
  )
}

/**
 * The composer hint while idle: the selected profile's example before the
 * first message, the generic hint otherwise (after the first turn the
 * example no longer describes a useful next step).
 */
export function idleComposerPlaceholder(
  agentProfile: AgentProfileSnapshot | undefined,
  hasMessages: boolean,
): string {
  if (hasMessages) return GENERIC_COMPOSER_PLACEHOLDER
  return (
    normalizeComposerPlaceholder(agentProfile?.composerPlaceholder) ??
    GENERIC_COMPOSER_PLACEHOLDER
  )
}
