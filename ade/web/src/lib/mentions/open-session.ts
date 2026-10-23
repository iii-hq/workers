/**
 * How a `{ session }` mention opens its chat. The conversations provider
 * registers its opener here, so pills rendered anywhere (markdown included)
 * can open a session without importing the conversations context — which
 * would close an import cycle through the injectable-UI loader.
 */

type SessionOpener = (sessionId: string) => void

let opener: SessionOpener | null = null

export function setMentionSessionOpener(next: SessionOpener): () => void {
  opener = next
  return () => {
    if (opener === next) opener = null
  }
}

/** Opens the chat; false when no conversations surface is mounted. */
export function openMentionSession(sessionId: string): boolean {
  if (!opener || !sessionId) return false
  opener(sessionId)
  return true
}
