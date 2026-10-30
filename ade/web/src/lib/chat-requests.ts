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

/**
 * Outside requests to open one of the chat pane's dialogs — the New chat
 * and Add project shortcuts, dispatched from the app shell. The latest
 * request waits for a pane when none is mounted yet, so a key pressed while
 * the chat page is still opening is not lost.
 */
export type ChatDialog = 'new-chat' | 'add-project'

type ChatDialogListener = (dialog: ChatDialog) => void

const dialogListeners = new Set<ChatDialogListener>()
let pendingDialog: ChatDialog | null = null

export function requestChatDialog(dialog: ChatDialog): void {
  if (dialogListeners.size === 0) {
    pendingDialog = dialog
    return
  }
  for (const listener of dialogListeners) listener(dialog)
}

export function onChatDialogRequest(listener: ChatDialogListener): () => void {
  dialogListeners.add(listener)
  if (pendingDialog !== null) {
    const dialog = pendingDialog
    pendingDialog = null
    listener(dialog)
  }
  return () => {
    dialogListeners.delete(listener)
  }
}
