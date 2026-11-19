import type { Host } from '@iii-dev/console-ui'

/**
 * The chat beside the tree. Every ticket and session the tree opens shows in ONE column to the right of the
 * Kanban pane: the conversation pinned there (`chat:<session>`), swapped in place for the next one
 * (`panels.openScreen({ replaces })`), so clicking ticket after ticket never piles up columns and the tree never
 * leaves the screen. The workspace's own chat column, wherever it is, is left alone.
 *
 * `beside` is this page's own screen (`ext:kanban-board` / `ext:kanban-projects`).
 */

/**
 * The side-chat column each Kanban page opened last (its screen), so the next one takes its place. Kept in this
 * browser: the workspace layout outlives a reload, and so must the column's identity, or a reload would add a
 * second side chat instead of reusing the first.
 */
const lastShown = {
  key: (beside: string) => `kanban:side-chat:${beside}`,
  get(beside: string): string | undefined {
    try {
      return localStorage.getItem(this.key(beside)) ?? undefined
    } catch {
      return undefined
    }
  },
  set(beside: string, screen: string) {
    try {
      localStorage.setItem(this.key(beside), screen)
    } catch {
      // Private mode: the next reload adds a column; nothing worse.
    }
  },
}

function showBeside(host: Host, beside: string, conversationId: string): boolean {
  const open = host.panels?.openScreen
  if (!open) return false
  const screen = `chat:${conversationId}`
  const previous = lastShown.get(beside)
  open({
    screen,
    relativeTo: beside,
    direction: 'right',
    ...(previous && previous !== screen ? { replaces: previous } : {}),
  })
  lastShown.set(beside, screen)
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
export function draftChatBeside(host: Host, beside: string, draft: { text: string; title?: string }): boolean {
  const chat = host.chat
  if (chat?.openDraft) {
    const id = chat.openDraft(draft, { show: false })
    if (typeof id === 'string' && id && showBeside(host, beside, id)) return true
    if (typeof id === 'string') chat.selectConversation?.(id)
    return true
  }
  if (chat?.compose) {
    chat.compose({ text: draft.text, beside })
    return true
  }
  return false
}
