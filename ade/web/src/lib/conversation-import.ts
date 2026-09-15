import { getIiiClient } from '@/lib/iii-client'

export const conversationSources = {
  codex: 'Codex',
  'claude-code': 'Claude Code',
} as const
export type ConversationSource = keyof typeof conversationSources
export type ExternalConversation = {
  id: string
  source: ConversationSource
  title: string
  cwd: string | null
  created_at: number
  updated_at: number
}
export type Discovery = {
  source: ConversationSource
  directory: string
  warnings: string[]
  available: boolean
  conversations: ExternalConversation[]
  next_cursor: string | null
}
export type ConversationPreview = {
  conversation: ExternalConversation
  messages: {
    id: string
    role: 'user' | 'assistant'
    text: string
    timestamp: number
  }[]
  warnings: string[]
}
export async function discoverConversations(input: {
  source: ConversationSource
  cursor?: string
  query?: string
}) {
  return (await getIiiClient()).trigger<Discovery>(
    'console::conversations::discover',
    input,
  )
}
export async function previewConversation(
  source: ConversationSource,
  id: string,
) {
  return (await getIiiClient()).trigger<ConversationPreview>(
    'console::conversations::preview',
    { source, id },
  )
}
export async function importConversation(
  source: ConversationSource,
  id: string,
) {
  return (await getIiiClient()).trigger<{
    session_id: string
    imported_messages: number
    total_messages: number
  }>('console::conversations::import', { source, id }, { timeoutMs: 120_000 })
}
