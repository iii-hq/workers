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
/** A tool the source's agent called, as the source recorded it. */
export type ExternalCall = {
  id: string
  /** The source's own tool name: `Bash`, `Read`, `exec`, `apply_patch`… */
  function_id: string
  arguments: unknown
}
export type PreviewMessage = {
  id: string
  role: 'user' | 'assistant' | 'function_result'
  text: string
  timestamp: number
  /** The calls an assistant turn made, in order. */
  calls?: ExternalCall[]
  /** On a `function_result`: the call it answers and the tool that ran. */
  call_id?: string
  function_id?: string
  is_error?: boolean
}
export type ConversationPreview = {
  conversation: ExternalConversation
  messages: PreviewMessage[]
  warnings: string[]
}
export type ImportResult = {
  session_id: string
  /** Every entry appended to the new session: turns, calls, and results. */
  imported_messages: number
  /** User and assistant turns that carried text. */
  total_messages: number
  /** Tool calls that came across with their recorded results. */
  imported_commands: number
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
  return (await getIiiClient()).trigger<ImportResult>(
    'console::conversations::import',
    { source, id },
    { timeoutMs: 120_000 },
  )
}
