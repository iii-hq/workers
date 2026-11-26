/**
 * "Conversation details" (chat-projects §1.3.5): what the dialog edits, and
 * the one `session::patch-meta` its Save writes.
 *
 * - A changed name sets `title_manual` (the agent never renames it, D13).
 * - A description typed by the user sets `description_by: "user"`,
 *   `description_at_ms`, `description_manual: true`, and pushes the old
 *   description into `description_history` (5 kept, newest first).
 * - The switch "Let the agent keep it up to date" writes
 *   `description_manual` (on = false).
 * - The project is not metadata the dialog writes: a changed project is a
 *   `harness::projects::assign`, which also moves subagents and sidechats.
 * - The Claude Code account (phase 4, §4.3.5) writes `claude_account`: an
 *   account id pins the conversation, `"global"` follows the global, and the
 *   project's choice removes the key.
 */
import type { Conversation } from '@/types/chat'

export const DESCRIPTION_MAX = 140
/** The counter shows from this many characters. */
export const DESCRIPTION_COUNTER_FROM = 120
export const DESCRIPTION_HISTORY_MAX = 5

export interface DescriptionHistoryEntry {
  text: string
  by: 'agent' | 'user'
  at_ms: number
}

/** The account select's first option: the project's account (no `claude_account`). */
export const ACCOUNT_DEFAULT = ':default'

export interface ConversationDetailsDraft {
  title: string
  description: string
  /** "Let the agent keep it up to date". */
  agentKeepsIt: boolean
  /** A project id, or `none`. */
  projectId: string
  /**
   * `metadata.claude_account`: an account id, `"global"`, or
   * `ACCOUNT_DEFAULT` for the project's. Absent (the draft's start) leaves
   * it alone; the row reads `claudeAccountOf` until it is chosen.
   */
  claudeAccount?: string
}

/** A conversation's `claude_account` as the select shows it. */
export function claudeAccountOf(
  metadata: Readonly<Record<string, unknown>> | undefined,
): string {
  const value = metadata?.claude_account
  return typeof value === 'string' && value.length > 0 ? value : ACCOUNT_DEFAULT
}

export interface DetailsPatch {
  title?: string
  description?: string
  set: Record<string, unknown>
  unset: string[]
}

/** One line, single spaces, at most 140 characters. */
export function cleanDescription(text: string): string {
  return text.replace(/\s+/g, ' ').trim().slice(0, DESCRIPTION_MAX)
}

/** `metadata.description_history`, defensively, newest first. */
export function descriptionHistory(
  metadata: Readonly<Record<string, unknown>> | undefined,
): DescriptionHistoryEntry[] {
  const raw = metadata?.description_history
  if (!Array.isArray(raw)) return []
  return raw
    .flatMap((entry) => {
      if (!entry || typeof entry !== 'object') return []
      const { text, by, at_ms } = entry as Record<string, unknown>
      if (typeof text !== 'string' || !text.trim()) return []
      return [
        {
          text,
          by: by === 'agent' ? ('agent' as const) : ('user' as const),
          at_ms: typeof at_ms === 'number' ? at_ms : 0,
        },
      ]
    })
    .slice(0, DESCRIPTION_HISTORY_MAX)
}

/** Whether the agent may write the description (`description_manual` is not true). */
export function agentKeepsDescription(
  conversation: Pick<Conversation, 'sessionMetadata'>,
): boolean {
  return conversation.sessionMetadata?.description_manual !== true
}

/** The dialog's starting values for a conversation. */
export function detailsDraftOf(
  conversation: Pick<
    Conversation,
    'title' | 'description' | 'projectId' | 'sessionMetadata'
  >,
): ConversationDetailsDraft {
  return {
    title: conversation.title,
    description: conversation.description ?? '',
    agentKeepsIt: agentKeepsDescription(conversation),
    projectId: conversation.projectId ?? 'none',
  }
}

/**
 * The metadata write for a Save, or null when nothing but (maybe) the
 * project changed.
 */
export function buildDetailsPatch(
  conversation: Pick<
    Conversation,
    | 'title'
    | 'description'
    | 'descriptionBy'
    | 'descriptionAtMs'
    | 'sessionMetadata'
  >,
  draft: ConversationDetailsDraft,
  now: number,
): DetailsPatch | null {
  const patch: DetailsPatch = { set: {}, unset: [] }
  const title = draft.title.trim()
  if (title && title !== conversation.title) {
    patch.title = title
    patch.set.title_manual = true
  }
  const before = conversation.description ?? ''
  const description = cleanDescription(draft.description)
  const descriptionChanged = description !== before
  if (descriptionChanged) {
    patch.description = description
    patch.set.description_by = 'user'
    patch.set.description_at_ms = now
    patch.set.description_manual = true
    const history = descriptionHistory(conversation.sessionMetadata)
    patch.set.description_history = (
      before
        ? [
            {
              text: before,
              by: conversation.descriptionBy ?? 'user',
              at_ms: conversation.descriptionAtMs ?? now,
            },
            ...history,
          ]
        : history
    ).slice(0, DESCRIPTION_HISTORY_MAX)
  }
  const manual = !draft.agentKeepsIt
  if (manual !== !agentKeepsDescription(conversation)) {
    patch.set.description_manual = manual
  } else if (descriptionChanged && draft.agentKeepsIt) {
    // Typed, then the switch was turned back on: the agent may write again.
    patch.set.description_manual = false
  }
  if (
    draft.claudeAccount !== undefined &&
    draft.claudeAccount !== claudeAccountOf(conversation.sessionMetadata)
  ) {
    if (draft.claudeAccount === ACCOUNT_DEFAULT)
      patch.unset.push('claude_account')
    else patch.set.claude_account = draft.claudeAccount
  }
  return patch.title !== undefined ||
    patch.description !== undefined ||
    Object.keys(patch.set).length > 0 ||
    patch.unset.length > 0
    ? patch
    : null
}

/** The patch applied to a metadata map (the `session::set-meta` fallback). */
export function applyPatchToMetadata(
  metadata: Readonly<Record<string, unknown>> | undefined,
  patch: Pick<DetailsPatch, 'set' | 'unset'>,
): Record<string, unknown> {
  const next: Record<string, unknown> = { ...(metadata ?? {}), ...patch.set }
  for (const key of patch.unset) delete next[key]
  return next
}
