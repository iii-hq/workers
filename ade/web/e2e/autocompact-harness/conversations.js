/*
 * The tab's conversations, as the auto-compact surfaces read them: the
 * overrides table counts the sessions with their own value, the history
 * opens a conversation, and a session row saves through Details.
 */
import { harness } from './mock-iii.js'

const conversation = (id, title, extra = {}) => ({
  id,
  title,
  model: null,
  messages: [],
  createdAt: 1,
  updatedAt: 1,
  sessionMetadata: {},
  ...extra,
})

const value = {
  conversations: [
    conversation('s-ledger', 'Ledger reconciliation', {
      projectId: 'prj_billing',
      sessionMetadata: { autocompact: { mode: 'tokens', tokens: 600_000 } },
    }),
    conversation('s-webhooks', 'Webhook retries', { projectId: 'prj_web' }),
    // A side chat's own copy never counts: it follows its conversation.
    conversation('s-side', 'Side question', {
      sessionMetadata: {
        side_chat_of: 's-ledger',
        autocompact: { mode: 'off' },
      },
    }),
  ],
  openConversation: (id) => harness.opened.push(id),
  saveDetails: async (id, draft) => {
    harness.saved.push({ id, draft })
  },
}

export function useConversationsCtxOptional() {
  return value
}

export function useConversationsCtx() {
  return value
}
