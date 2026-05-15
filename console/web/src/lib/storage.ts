import { type Conversation, isKnownRole, type Message } from '@/types/chat'

const CONVERSATIONS_KEY = 'iii-chat-conversations'
const ACTIVE_KEY = 'iii-chat-active'

export function loadConversations(): Conversation[] {
  try {
    const raw = localStorage.getItem(CONVERSATIONS_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(isConversation).map((c) => ({
      ...c,
      messages: c.messages.filter(isValidMessage),
    }))
  } catch {
    return []
  }
}

export function saveConversations(list: Conversation[]): void {
  try {
    localStorage.setItem(CONVERSATIONS_KEY, JSON.stringify(list))
  } catch {
    /* quota or private mode: best-effort */
  }
}

export function loadActiveId(): string | null {
  try {
    return localStorage.getItem(ACTIVE_KEY)
  } catch {
    return null
  }
}

export function saveActiveId(id: string | null): void {
  try {
    if (id) localStorage.setItem(ACTIVE_KEY, id)
    else localStorage.removeItem(ACTIVE_KEY)
  } catch {
    /* best-effort */
  }
}

function isConversation(v: unknown): v is Conversation {
  if (!v || typeof v !== 'object') return false
  const c = v as Record<string, unknown>
  return (
    typeof c.id === 'string' &&
    typeof c.title === 'string' &&
    Array.isArray(c.messages) &&
    typeof c.createdAt === 'number' &&
    typeof c.updatedAt === 'number'
  )
}

function isValidMessage(v: unknown): v is Message {
  if (!v || typeof v !== 'object') return false
  const m = v as Record<string, unknown>
  return (
    typeof m.id === 'string' &&
    typeof m.createdAt === 'number' &&
    isKnownRole(m.role)
  )
}
