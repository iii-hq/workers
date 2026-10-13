/**
 * Outside requests to show one chat: a setup wizard example, a page that
 * started a turn, a `@session` pill. A chat pane on its narrow list page
 * follows them into its chat page; a change of the active chat alone (a
 * deletion moving on to the next chat) does not move it.
 */
type ChatShownListener = (conversationId: string) => void

const listeners = new Set<ChatShownListener>()

export function requestChatShown(conversationId: string): void {
  for (const listener of listeners) listener(conversationId)
}

export function onChatShownRequest(listener: ChatShownListener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}
