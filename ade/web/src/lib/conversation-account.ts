/**
 * The Claude Code account a conversation runs on, as the engine decides it
 * (harness/src/accounts.rs, `resolve_turn`). The limit card, the account chip
 * and Conversation details' row all read this one resolution:
 * - a side chat runs on the account its lock froze from its conversation's
 *   turn (`metadata.side_chat_lock.account`, D80); without one (its
 *   conversation had no turn yet) it resolves as a conversation;
 * - a sub-agent runs on its parent's current turn account (`inherited()`):
 *   the parent's own resolution when the parent is loaded here, else the
 *   parent turn's account from the verbose `harness::status`;
 * - any other conversation: `metadata.claude_account`, its project's pin,
 *   the global (§4.2.4).
 */
import {
  type ChatProject,
  NO_PROJECT,
  parseProjectAccount,
} from '@/lib/backend/projects'
import { isSideChat, sideChatLock } from '@/lib/chat-aside'
import {
  type AccountResolution,
  type AccountSource,
  type LockedAccount,
  resolveAccount,
} from '@/lib/claude-accounts-words'
import { ACCOUNT_DEFAULT, claudeAccountOf } from '@/lib/conversation-details'
import type { Conversation } from '@/types/chat'

/**
 * Where a conversation's account comes from (§4.3.5): the text the row and
 * the chip read, from the draft's choice (`ACCOUNT_DEFAULT`, `"global"` or
 * an id), its project and the global account.
 */
export function conversationResolution(input: {
  choice: string
  project: Pick<ChatProject, 'name' | 'account'> | null
  global: string | null
}): AccountResolution {
  return resolveAccount({
    claudeAccount: input.choice === ACCOUNT_DEFAULT ? undefined : input.choice,
    project: input.project
      ? {
          name: input.project.name,
          account: parseProjectAccount(input.project.account),
        }
      : null,
    global: input.global,
  })
}

type AccountConversation = Pick<
  Conversation,
  'sessionMetadata' | 'parentId' | 'projectId'
>

/** A side chat (shown as one, or its metadata says so), a sub-agent, or neither. */
export function lockedAccountOf(
  conversation: Pick<Conversation, 'sessionMetadata' | 'parentId'>,
  sideChat = false,
): LockedAccount {
  if (sideChat || isSideChat(conversation)) return 'side-chat'
  return conversation.parentId ? 'sub-agent' : null
}

/** The parent turn's account from the verbose `harness::status`. */
export interface TurnAccountRef {
  id: string
  source?: string
}

const SOURCES: readonly AccountSource[] = ['session', 'project', 'global']

/** Sub-agents of sub-agents: far deeper than any real tree, never a loop. */
const MAX_DEPTH = 16

export function chatAccountResolution(input: {
  conversation: AccountConversation
  /** Shown as a side chat (ChatView's `side`), whatever its metadata says. */
  sideChat?: boolean
  projects: readonly Pick<ChatProject, 'id' | 'name' | 'account'>[]
  global: string | null
  /** The conversations loaded here, where a sub-agent's parent is looked for. */
  conversations?: readonly (AccountConversation & { id: string })[]
  /** The parent turn's account, for a sub-agent whose parent isn't loaded. */
  parentTurnAccount?: TurnAccountRef | null
}): AccountResolution {
  let conversation = input.conversation
  let sideChat = input.sideChat ?? false
  let sub = false
  for (let depth = 0; depth < MAX_DEPTH; depth++) {
    const locked = lockedAccountOf(conversation, sideChat)
    if (locked === 'side-chat') {
      const account = sideChatLock(conversation)?.account
      if (account) {
        return {
          account,
          source: 'session',
          locked: sub ? 'sub-agent' : locked,
        }
      }
      break
    }
    if (locked !== 'sub-agent') break
    const parent = input.conversations?.find(
      (c) => c.id === conversation.parentId,
    )
    if (!parent) {
      const turn = depth === 0 ? input.parentTurnAccount : null
      if (turn?.id) {
        const source = SOURCES.find((s) => s === turn.source) ?? 'session'
        return { account: turn.id, source, locked: 'sub-agent' }
      }
      // Nothing to read the parent's account from: the sub-agent's own.
      break
    }
    conversation = parent
    sideChat = false
    sub = true
  }
  const projectId = conversation.projectId
  const project =
    projectId && projectId !== NO_PROJECT
      ? (input.projects.find((p) => p.id === projectId) ?? null)
      : null
  const own = conversationResolution({
    choice: claudeAccountOf(conversation.sessionMetadata),
    project,
    global: input.global,
  })
  return sub ? { ...own, locked: 'sub-agent' } : own
}
