// Showing a session's conversation from the monitor: the observed session of
// an analysis, or the analyst's own. No React.
import type { Host } from '@iii-dev/console-ui'

/** Whether this console can show a conversation at all; controls hide when it cannot. */
export function canOpenSession(host: Host): boolean {
  return Boolean(host.panels?.openScreen || host.chat?.selectConversation)
}

/**
 * Opens the conversation in a column beside this page; a console that does not
 * know the `chat:<id>` screen throws, and the conversation is selected instead.
 * Returns whether either was invoked.
 */
export function openSession(host: Host, sessionId: string): boolean {
  if (host.panels?.openScreen) {
    try {
      host.panels.openScreen({ screen: `chat:${sessionId}` })
      return true
    } catch {
      // Unknown screen on this console: fall through to selecting it.
    }
  }
  if (host.chat?.selectConversation) {
    host.chat.selectConversation(sessionId)
    return true
  }
  return false
}
