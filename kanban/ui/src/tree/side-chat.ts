import type { Host } from '@iii-dev/console-ui'

/**
 * The chat beside the tree. Every ticket and session the tree opens shows in ONE column to the right of the
 * Kanban pane: the session chat (`chat:<session>`) directly beside this page takes the next one
 * (`panels.openScreen({ reuseBeside })`), so clicking ticket after ticket never piles up columns and the tree never
 * leaves the screen. The column is found from the shared layout itself, so another browser, or this one after a
 * reload, reuses it too. The workspace's own chat column (`chat`) is never taken.
 *
 * `beside` is this page's own screen (`ext:kanban-board` / `ext:kanban-projects`).
 */

function showBeside(host: Host, beside: string, conversationId: string, draft?: { text: string; title?: string }) {
  const open = host.panels?.openScreen
  if (!open) return false
  open({
    screen: `chat:${conversationId}`,
    relativeTo: beside,
    direction: 'right',
    reuseBeside: true,
    // An unsent draft travels with the layout: a reload or another browser opens the same draft, not a missing chat.
    ...(draft ? { draft } : {}),
  })
  return true
}

/** Show `sessionId` in the chat beside the tree. False when this console has no workspace to show it in. */
export function openChatBeside(host: Host, beside: string, sessionId: string): boolean {
  if (showBeside(host, beside, sessionId)) return true
  if (host.chat?.selectConversation) {
    host.chat.selectConversation(sessionId)
    return true
  }
  return false
}

/**
 * Start a new chat beside the tree with `text` in its composer (not sent): about a ticket, or the project. On a
 * console whose `openDraft` does not return the draft's id, the draft opens in the main chat instead.
 */
export function draftChatBeside(
  host: Host,
  beside: string,
  draft: { text: string; title?: string },
  /** The new chat's id, when the console says it. */
  onDraftId?: (id: string) => void,
): boolean {
  const chat = host.chat
  if (chat?.openDraft) {
    // Hidden only when something can then show it: beside the page, or as the main chat. Otherwise it opens visibly.
    if (!host.panels?.openScreen && !chat.selectConversation) {
      chat.openDraft(draft)
      return true
    }
    const id = chat.openDraft(draft, { show: false })
    if (typeof id === 'string' && id) onDraftId?.(id)
    if (typeof id === 'string' && id && showBeside(host, beside, id, draft)) return true
    if (typeof id === 'string' && id) chat.selectConversation?.(id)
    else chat.openDraft(draft)
    return true
  }
  if (chat?.compose) {
    chat.compose({ text: draft.text, beside })
    return true
  }
  return false
}
