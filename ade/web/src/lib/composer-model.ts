/**
 * What `host.chat.composerModel` answers: the model the chat would send
 * with. That is the conversation's own pick when it has one, else the model a
 * new chat starts on (the pinned default, the last pick, the provider's
 * default, the router's first model) — the same model the composer shows.
 *
 * Answering `null` for a chat that simply has not picked yet left pages that
 * start their own sessions (Cron's "Create schedule") unable to send for a
 * new user whose composer plainly showed a model, and made them flip to
 * "choose a model" the moment they selected the session they had just made.
 */
export function composerModelFor(
  conversations: readonly { id: string; model?: unknown }[],
  conversationId: string | null | undefined,
  startingModel: string | null | undefined,
): string | null {
  const own = conversationId
    ? conversations.find((conversation) => conversation.id === conversationId)
        ?.model
    : undefined
  if (typeof own === 'string' && own.trim()) return own.trim()
  return startingModel?.trim() || null
}
