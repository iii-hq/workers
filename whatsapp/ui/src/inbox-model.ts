export interface MessageView {
  message_id: string
  chat_id: string
  sender_id: string
  sender_name?: string | null
  from_me: boolean
  timestamp_ms: number
  kind: string
  text: string
  source: 'live' | 'history' | 'sent'
  status?: string | null
}

export interface ChatTarget { chat_id: string; title: string; is_group: boolean }
export interface ChatView extends ChatTarget { last_message?: MessageView | null; message_count: number }
export interface Coverage {
  capture_started_at_ms: number
  last_history_sync_at_ms?: number | null
  last_history_sync_progress?: number | null
  message_limit: number
  retention_truncated: boolean
}
export interface ChatPage { chats: ChatView[]; next_cursor: string | null; coverage: Coverage }
export interface MessagePage { messages: MessageView[]; next_cursor: string | null; coverage: Coverage }

export function messageKey(message: MessageView): string {
  return JSON.stringify([message.from_me ? 'me' : message.sender_id, message.message_id])
}

export function mergeMessages(previous: MessageView[], incoming: MessageView[], chatId: string): MessageView[] {
  const messages = new Map<string, MessageView>()
  for (const message of [...previous, ...incoming]) {
    if (message.chat_id === chatId) messages.set(messageKey(message), message)
  }
  return [...messages.values()].sort((a, b) => a.timestamp_ms - b.timestamp_ms || a.message_id.localeCompare(b.message_id) || Number(a.from_me) - Number(b.from_me))
}

export function mergeChats(previous: ChatView[], incoming: ChatView[]): ChatView[] {
  const chats = new Map(previous.map((chat) => [chat.chat_id, chat]))
  for (const chat of incoming) chats.set(chat.chat_id, chat)
  return [...chats.values()].sort((a, b) => (b.last_message?.timestamp_ms ?? 0) - (a.last_message?.timestamp_ms ?? 0) || a.title.localeCompare(b.title))
}

const KIND_LABELS: Record<string, string> = {
  image: 'Image', video: 'Video', audio: 'Audio', document: 'Document', sticker: 'Sticker',
  contact: 'Contact', location: 'Location', reaction: 'Reaction', poll: 'Poll', unknown: 'Message',
  edited: 'Edited message', revoked: 'Deleted message', deleted: 'Deleted message',
  expired: 'Expired message', ephemeral: 'Disappearing message', view_once: 'View-once message',
  system: 'Notice', text: 'Message without text',
}
export function messageLabel(message: MessageView): string {
  return message.text?.trim() ? message.text : KIND_LABELS[message.kind] ?? 'Message without a preview'
}

export function isMediaMessage(message: MessageView): boolean {
  return ['image', 'video', 'audio', 'document', 'sticker'].includes(message.kind)
}

export function identityLabel(jid: string): string {
  if (jid.endsWith('@s.whatsapp.net') || jid.endsWith('@c.us')) return `+${jid.split('@')[0]}`
  return jid
}

export function coverageText(coverage: Coverage): string {
  const received = coverage.last_history_sync_at_ms
    ? 'Captured messages and history received from WhatsApp.'
    : 'Messages captured in this session; awaiting available history from WhatsApp.'
  return coverage.retention_truncated
    ? `${received} Older messages were removed because of the ${coverage.message_limit.toLocaleString('en-US')}-message limit per session.`
    : received
}

export function messageStatus(status: string | null | undefined): string {
  if (!status) return ''
  const known: Record<string, string> = {
    sent: 'Sent', submitted: 'Submitted', delivered: 'Delivered', delivery: 'Delivered',
    read: 'Read', played: 'Played', pending: 'Pending', error: 'Send failed', failed: 'Send failed',
    server_ack: 'Received by the server',
  }
  return known[status] ?? status
}
