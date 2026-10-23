import { useDraftRegistration, useDraftScope, useDraftState, usePageState } from './drafts'
import { Button, EmptyState, IconButton, Input, List, ListItem, LiveRegion, PageBody, PageMain, PageSidebar, SearchField, Skeleton, StatusPanel, type Host, type LiveAnnouncement } from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import { ArrowLeft, Check, ChevronDown, ChevronUp, History, Info, MessageCircle, MessageSquarePlus, Paperclip, PencilLine, RefreshCw, Send, UserRound, Users, WifiOff } from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { coverageText, identityLabel, isMediaMessage, mergeChats, mergeMessages, messageKey, messageLabel, messageStatus, type ChatPage, type ChatTarget, type Coverage, type MessagePage, type MessageView } from './inbox-model'
import { normalizeRecipient, requestScope, validateMessage, type SentMessage } from './model'
import { useInboxPages } from './use-inbox'

const dateLabel = (timestamp: number) => new Date(timestamp).toLocaleDateString('en-US', { day: 'numeric', month: 'long', year: 'numeric' })
const timeLabel = (timestamp: number) => new Date(timestamp).toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' })

export function ChatIdentity({ chat, header = false }: { chat: ChatTarget; header?: boolean }) {
  const name = chat.title.trim()
  const initials = name && name !== chat.chat_id && name !== identityLabel(chat.chat_id) && !/^\+?[\d\s()-]+$/.test(name)
    ? name.split(/\s+/).slice(0, 2).map((part) => Array.from(part)[0]).join('').toLocaleUpperCase('en-US')
    : null
  return <span className={`wa-ui-avatar${header ? ' wa-ui-avatar-header' : ''}`} data-group={chat.is_group || undefined} aria-hidden="true">{chat.is_group ? <Users size={16} /> : initials || <UserRound size={16} />}</span>
}

function ArchiveCoverage({ coverage, count }: { coverage: Coverage; count?: number }) {
  return <details className="wa-ui-archive-details">
    <summary><Info size={16} aria-hidden="true" /><span>{count === undefined ? 'Archive availability' : `${count.toLocaleString('en-US')} messages shown`}</span><ChevronDown size={16} aria-hidden="true" /></summary>
    <p className="wa-ui-archive-copy wa-ui-note">{coverageText(coverage)} To request older history, connect the account and open a conversation with at least one stored message.</p>
  </details>
}

export function MessageBrowser({ host, sessionId, connected, narrow, panelSide, selected, onSelect, onDirty, onGoConnection }: {
  host: Host; sessionId: string; connected: boolean; narrow: boolean; panelSide?: 'left' | 'right'
  selected: ChatTarget | null; onSelect(chat: ChatTarget | null): Promise<boolean>; onDirty(dirty: boolean): void; onGoConnection(): void
}) {
  const [query, setQuery] = useState('')
  const [newChat, setNewChat] = useState(false)
  const [recipient, setRecipient] = useState('')
  const [recipientError, setRecipientError] = useState<string | null>(null)
  const search = useDebounce(query, 250)
  const list = useInboxPages({
    identity: `${sessionId}:${search}`,
    fetchPage: async (cursor?: string) => {
      const page = await host.iii.trigger<ChatPage>('whatsapp::chats::list', { session_id: sessionId, query: search || undefined, cursor, limit: 100 })
      return { ...page, items: page.chats }
    },
    merge: mergeChats,
  })
  const openRecipient = async (event: React.FormEvent) => {
    event.preventDefault()
    try {
      const chatId = normalizeRecipient(recipient)
      if (!await onSelect({ chat_id: chatId, title: identityLabel(chatId), is_group: chatId.endsWith('@g.us') })) return
      setRecipient('')
      setRecipientError(null)
      setNewChat(false)
    } catch (cause) { setRecipientError(errorMessage(cause)) }
  }

  return <PageBody side={panelSide} className="wa-ui-browser">
    {(!narrow || !selected) && <PageSidebar label="Conversations" side={panelSide} narrow={narrow} defaultWidth={288} minWidth={240} maxWidth={400} header={<div className="wa-ui-sidebar-heading"><span>Stored conversations</span><div className="wa-ui-header-actions"><IconButton label={newChat ? 'Close new conversation' : 'New conversation'} aria-expanded={newChat} onClick={() => setNewChat((value) => !value)}><MessageSquarePlus size={16} /></IconButton><IconButton label="Refresh conversations" onClick={list.refresh} disabled={list.loading}><RefreshCw size={16} /></IconButton></div></div>}>
      <div className="wa-ui-list-tools">
        <SearchField aria-label="Search conversations" placeholder="Search conversations" value={query} onChange={setQuery} />
        <span className="wa-ui-note">{list.items.length.toLocaleString('en-US')} {list.items.length === 1 ? 'conversation' : 'conversations'}</span>
        {!connected && <div className="wa-ui-archive-offline"><p className="wa-ui-note">Offline · stored history</p><Button variant="ghost" onClick={onGoConnection}>Connect</Button></div>}
        {newChat && <form onSubmit={openRecipient} className="wa-ui-stack">
          <Input autoFocus className="wa-ui-mono" aria-label="New conversation recipient" value={recipient} onChange={(value) => { setRecipient(value); setRecipientError(null) }} placeholder="Phone number or group ID" autoComplete="off" />
          {recipientError && <StatusPanel role="alert" variant="alert" headline={recipientError} />}
          <Button type="submit" disabled={!recipient.trim()}>Open conversation</Button>
        </form>}
      </div>
      {list.error && <div className="wa-ui-list-tools"><StatusPanel role="alert" variant="alert" headline="Could not load conversations" detail={list.error} action={<Button variant="ghost" onClick={list.refresh}>Try again</Button>} /></div>}
      <div className="wa-ui-list">
        {list.loading ? <><Skeleton className="wa-ui-loading" /><Skeleton className="wa-ui-loading" /></> : list.items.length === 0 ? <EmptyState compact icon={MessageCircle} title={query ? 'No conversations found' : 'No conversations yet'} description={query ? 'Try another name or identifier.' : 'Received messages and available WhatsApp history will appear here.'} /> : <List aria-label="Available conversations">{list.items.map((chat) => <ListItem key={chat.chat_id} className="wa-ui-chat-item" selected={selected?.chat_id === chat.chat_id}
          leading={<ChatIdentity chat={chat} />}
          label={<span className="wa-ui-chat-heading"><span className={`wa-ui-chat-title${!chat.title || chat.title === chat.chat_id || chat.title === identityLabel(chat.chat_id) || /^\+?[\d\s()-]+$/.test(chat.title) ? ' wa-ui-mono' : ''}`} title={chat.title || identityLabel(chat.chat_id)}>{chat.title || identityLabel(chat.chat_id)}</span>{chat.last_message && <time className="wa-ui-chat-meta" dateTime={new Date(chat.last_message.timestamp_ms).toISOString()} title={new Date(chat.last_message.timestamp_ms).toLocaleString('en-US')}>{formatRelative(chat.last_message.timestamp_ms)}</time>}</span>}
          description={<span className="wa-ui-preview">{chat.last_message ? `${chat.last_message.from_me ? 'You: ' : ''}${messageLabel(chat.last_message)}` : 'No message preview'}</span>}
          onClick={() => onSelect(chat)} />)}</List>}
        {list.nextCursor && <div className="wa-ui-list-tools"><Button variant="ghost" onClick={list.more} disabled={list.loadingMore}>{list.loadingMore ? 'Loading…' : 'Load more conversations'}</Button></div>}
      </div>
      {list.coverage && <ArchiveCoverage coverage={list.coverage} />}
    </PageSidebar>}
    {(!narrow || selected) && <PageMain className="wa-ui-reader">
      {selected ? <MessageReader key={`${sessionId}:${selected.chat_id}`} host={host} sessionId={sessionId} chat={selected} connected={connected} narrow={narrow} onBack={() => onSelect(null)} onDirty={onDirty} onGoConnection={onGoConnection} refreshChats={list.refresh} /> : <EmptyState icon={connected ? MessageCircle : WifiOff} title={connected ? 'Stored conversations' : 'Session is offline'} description={connected ? 'Select a conversation to inspect messages and delivery updates.' : 'These are stored messages. Connect the session before testing live sends and replies.'} action={{ label: 'Test connection', onClick: onGoConnection }} />}
    </PageMain>}
  </PageBody>
}

export function MessageRow({ message, group }: { message: MessageView; group: boolean }) {
  const status = messageStatus(message.status)
  return <article className="wa-ui-message-row" data-outgoing={message.from_me || undefined} aria-label={`${message.from_me ? 'You' : message.sender_name || identityLabel(message.sender_id)}, ${timeLabel(message.timestamp_ms)}`}>
    <div className="wa-ui-message-bubble">
      {group && !message.from_me && <span className={`wa-ui-message-sender${!message.sender_name || message.sender_name === message.sender_id || message.sender_name === identityLabel(message.sender_id) ? ' wa-ui-mono' : ''}`}>{message.sender_name || identityLabel(message.sender_id)}</span>}
      <p className="wa-ui-message-text">{messageLabel(message)}</p>
      {isMediaMessage(message) && <span className="wa-ui-media-note"><Paperclip size={16} aria-hidden="true" />Media not downloaded</span>}
      <footer className="wa-ui-message-meta"><time dateTime={new Date(message.timestamp_ms).toISOString()} title={new Date(message.timestamp_ms).toLocaleString('en-US')}>{timeLabel(message.timestamp_ms)}</time>{message.source === 'history' && <span>History</span>}{message.from_me && status && <span className="wa-ui-message-delivery"><Check size={16} aria-hidden="true" />{status}</span>}</footer>
    </div>
  </article>
}

function MessageReader({ host, sessionId, chat, connected, narrow, onBack, onDirty, onGoConnection, refreshChats }: {
  host: Host; sessionId: string; chat: ChatTarget; connected: boolean; narrow: boolean; onBack(): void
  onDirty(dirty: boolean): void; onGoConnection(): void; refreshChats(): void
}) {
  const archive = useInboxPages<MessageView>({
    identity: `${sessionId}:${chat.chat_id}`,
    fetchPage: async (cursor?: string) => {
      const page = await host.iii.trigger<MessagePage>('whatsapp::messages::list', { session_id: sessionId, chat_id: chat.chat_id, cursor, limit: 50 })
      return { ...page, items: page.messages }
    },
    merge: (previous, incoming) => mergeMessages(previous, incoming, chat.chat_id),
  })
  const draftKey = `${useDraftScope() || `native:${sessionId}`}:conversation:${chat.chat_id}`
  const [compose, setCompose, clearDraft] = useDraftState(draftKey, { text: '' }, (value) => !!value.text.trim())
  const [attempt, setAttempt] = usePageState(`${draftKey}:send-error`, { error: null as string | null, uncertain: false })
  const draft = compose.text, sendError = attempt.error
  const setDraft = (text: string) => setCompose((previous) => ({ ...previous, text }))
  const setSendError = (error: string | null, uncertain = false) => setAttempt({ error, uncertain })
  const [offlineDraftOpen, setOfflineDraftOpen] = useState(false)
  const [sending, setSending] = useState(false)
  const [requesting, setRequesting] = useState(false)
  const [historyError, setHistoryError] = useState<string | null>(null)
  const [historyNotice, setHistoryNotice] = useState<string | null>(null)
  const [sent, setSent] = useState<SentMessage | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const sendBusy = useRef(false)
  const historyBusy = useRef(false)
  const scope = useRef(requestScope())
  const historyScope = useRef(requestScope())
  const scroll = useRef<HTMLDivElement>(null)
  const followLatest = useRef(true)
  const olderAnchor = useRef<{ height: number; top: number } | null>(null)
  const readingAnchor = useRef<{ element: HTMLElement; offset: number } | null>(null)
  const readerSize = useRef<{ width: number; height: number } | null>(null)
  const composer = useRef<HTMLTextAreaElement>(null)
  const instance = useId()
  useDraftRegistration(draftKey, { dirty: !!draft.trim(), busy: sending, clear: clearDraft })
  const draftVisible = connected || offlineDraftOpen || !!draft || sending || !!sendError

  const rememberPosition = () => {
    const node = scroll.current
    if (!node) return
    const size = readerSize.current
    // A resize can dispatch scroll before its observer restores the reading anchor.
    if (size && (size.width !== node.clientWidth || size.height !== node.clientHeight)) return
    followLatest.current = node.scrollHeight - node.scrollTop - node.clientHeight < 96
    const top = node.getBoundingClientRect().top
    const first = !followLatest.current && Array.from(node.querySelectorAll<HTMLElement>('.wa-ui-message-entry')).find((entry) => entry.getBoundingClientRect().bottom > top)
    readingAnchor.current = first ? { element: first, offset: first.getBoundingClientRect().top - top } : null
  }

  useEffect(() => () => { scope.current.invalidate(); historyScope.current.invalidate() }, [])
  useEffect(() => { if (offlineDraftOpen) composer.current?.focus() }, [offlineDraftOpen])
  useEffect(() => { onDirty(!!draft.trim() || sending); return () => onDirty(false) }, [draft, sending, onDirty])
  useLayoutEffect(() => {
    const node = scroll.current
    if (!node) return
    if (olderAnchor.current) {
      node.scrollTop = olderAnchor.current.top + node.scrollHeight - olderAnchor.current.height
      olderAnchor.current = null
    } else if (followLatest.current) node.scrollTop = node.scrollHeight
    else {
      const anchor = readingAnchor.current
      if (anchor && node.contains(anchor.element)) node.scrollTop += anchor.element.getBoundingClientRect().top - node.getBoundingClientRect().top - anchor.offset
    }
  }, [archive.items])
  useEffect(() => {
    const node = scroll.current
    if (!node) return
    const resize = new ResizeObserver(() => {
      readerSize.current = { width: node.clientWidth, height: node.clientHeight }
      if (olderAnchor.current) return
      if (followLatest.current) node.scrollTop = node.scrollHeight
      else {
        const anchor = readingAnchor.current
        if (anchor && node.contains(anchor.element)) node.scrollTop += anchor.element.getBoundingClientRect().top - node.getBoundingClientRect().top - anchor.offset
      }
    })
    resize.observe(node)
    return () => resize.disconnect()
  }, [])

  const send = async () => {
    if (!connected || sendBusy.current || historyBusy.current) return
    const invalid = validateMessage(draft)
    if (invalid) { setSendError(invalid); return }
    sendBusy.current = true
    const current = scope.current.begin()
    setSending(true)
    setSendError(null)
    setSent(null)
    try {
      const result = await host.iii.trigger<SentMessage>('whatsapp::messages::send_text', { session_id: sessionId, chat_id: chat.chat_id, text: draft }, { timeoutMs: 610000 })
      if (!current()) return
      clearDraft()
      setSent(result)
      followLatest.current = true
      archive.refresh()
      refreshChats()
      setAnnouncement({ seq: Date.now(), text: 'Message submitted.', urgency: 'polite' })
    } catch (cause) {
      if (current()) setSendError(`${errorMessage(cause)} Check the conversation before sending again. Sending again may duplicate the previous message; it will not be repeated automatically.`, true)
    } finally { if (current()) { sendBusy.current = false; setSending(false) } }
  }
  const requestHistory = async () => {
    if (!connected || historyBusy.current || sendBusy.current || archive.items.length === 0) return
    historyBusy.current = true
    const current = historyScope.current.begin()
    setRequesting(true)
    setHistoryError(null)
    try {
      const result = await host.iii.trigger<{ request_id: string; requested_count: number }>('whatsapp::messages::request_history', { session_id: sessionId, chat_id: chat.chat_id, count: 50 }, { timeoutMs: 610000 })
      if (!current()) return
      setHistoryNotice(`Up to ${result.requested_count} older messages requested. Available history will appear when WhatsApp responds.`)
      archive.refresh()
    } catch (cause) { if (current()) setHistoryError(errorMessage(cause)) }
    finally { if (current()) { historyBusy.current = false; setRequesting(false) } }
  }
  const older = () => {
    if (scroll.current) olderAnchor.current = { height: scroll.current.scrollHeight, top: scroll.current.scrollTop }
    archive.more()
  }
  const sendAction = <Button type="button" variant={attempt.uncertain ? 'ghost' : undefined} onClick={() => void send()} disabled={!connected || sending || requesting || !draft.trim()}><Send size={16} aria-hidden="true" />{sending ? 'Sending…' : attempt.uncertain ? 'Send again' : 'Send'}</Button>

  return <>
    <header className="wa-ui-detail-header wa-ui-compact-header">
      {narrow && <IconButton label="Back to conversations" onClick={onBack}><ArrowLeft size={16} /></IconButton>}
      <ChatIdentity chat={chat} header />
      <div className="wa-ui-heading-copy"><h2 className={`wa-ui-title${!chat.title || chat.title === chat.chat_id || chat.title === identityLabel(chat.chat_id) || /^\+?[\d\s()-]+$/.test(chat.title) ? ' wa-ui-mono' : ''}`} title={chat.title || identityLabel(chat.chat_id)}>{chat.title || identityLabel(chat.chat_id)}</h2><span className="wa-ui-note wa-ui-identity" title={chat.chat_id}>{chat.is_group ? 'Group · ' : ''}<span className="wa-ui-mono">{identityLabel(chat.chat_id)}</span></span></div>
      <div className="wa-ui-header-actions">
        <IconButton label="Refresh messages" onClick={archive.refresh} disabled={archive.loading}><RefreshCw size={16} /></IconButton>
        <IconButton label={requesting ? 'Requesting older messages' : 'Request older messages'} tooltip={!connected ? 'Connect this account to request older messages.' : archive.items.length === 0 ? 'A stored message is needed before requesting older history.' : 'Request up to 50 older messages from your phone.'} onClick={() => void requestHistory()} disabled={!connected || requesting || sending || archive.items.length === 0} aria-busy={requesting}><History size={16} /></IconButton>
      </div>
    </header>
    {archive.error && <div className="wa-ui-reader-notice"><StatusPanel role="alert" variant="alert" headline="Could not refresh messages" detail={archive.error} action={<Button variant="ghost" onClick={archive.refresh}>Try again</Button>} /></div>}
    {historyError && <div className="wa-ui-reader-notice"><StatusPanel role="alert" variant="alert" headline="Could not request history" detail={historyError} action={<Button variant="ghost" onClick={() => setHistoryError(null)}>Dismiss</Button>} /></div>}
    {historyNotice && <div className="wa-ui-reader-notice"><StatusPanel role="status" variant="info" headline="History requested" detail={historyNotice} action={<Button variant="ghost" onClick={() => setHistoryNotice(null)}>Dismiss</Button>} /></div>}
    <div ref={scroll} className="wa-ui-messages" onScroll={rememberPosition}>
      {archive.loading ? <><Skeleton className="wa-ui-loading" /><Skeleton className="wa-ui-loading" /></> : archive.items.length === 0 ? <EmptyState compact icon={MessageCircle} title="No messages stored yet" description="Messages captured here and received WhatsApp history will appear in this conversation." /> : <>
        {archive.nextCursor && <div className="wa-ui-load-older"><Button variant="ghost" onClick={older} disabled={archive.loadingMore}>{archive.loadingMore ? 'Loading…' : 'Load older messages'}</Button></div>}
        <div className="wa-ui-message-thread" role="log" aria-label={`Messages in ${chat.title || identityLabel(chat.chat_id)}`} aria-live="off">{archive.items.map((message, index) => <div key={messageKey(message)} className="wa-ui-message-entry">
          {(index === 0 || dateLabel(archive.items[index - 1].timestamp_ms) !== dateLabel(message.timestamp_ms)) && <div className="wa-ui-message-date">{dateLabel(message.timestamp_ms)}</div>}
          <MessageRow message={message} group={chat.is_group} />
        </div>)}</div>
      </>}
      {narrow && archive.coverage && <ArchiveCoverage coverage={archive.coverage} count={archive.items.length} />}
    </div>
    {!narrow && archive.coverage && <ArchiveCoverage coverage={archive.coverage} count={archive.items.length} />}
    <section className="wa-ui-composer" data-offline={!connected || undefined} aria-label="Write a message">
      {!connected && <div className="wa-ui-inline-status wa-ui-offline-band" data-tone="warn" role="status"><WifiOff size={16} aria-hidden="true" /><span>{draft ? 'Offline · draft kept' : 'Offline · stored history'}</span><Button variant="ghost" onClick={onGoConnection}>Connect</Button>{!draft && !sending && !sendError && <IconButton label={offlineDraftOpen ? 'Hide draft editor' : 'Write a draft'} aria-expanded={draftVisible} aria-controls={`${instance}-draft`} onClick={() => setOfflineDraftOpen((value) => !value)}>{offlineDraftOpen ? <ChevronUp size={16} /> : <PencilLine size={16} />}</IconButton>}</div>}
      {sendError && <StatusPanel className="wa-ui-feedback" role="alert" variant="alert" headline={attempt.uncertain ? 'Message not confirmed' : 'Message not sent'} detail={sendError} action={!attempt.uncertain ? <Button variant="ghost" onClick={() => setSendError(null)}>Dismiss</Button> : undefined} />}
      <div id={`${instance}-draft`} className="wa-ui-composer-surface" hidden={!draftVisible}>
        <div className="wa-ui-composer-heading"><label htmlFor={`${instance}-composer`} className="wa-ui-composer-label">Message</label><span id={`${instance}-shortcut`} className="wa-ui-note">{sent ? 'Submitted. Delivery updates in the conversation.' : attempt.uncertain ? 'Check the conversation before sending again.' : <>Text message<span className="wa-ui-cloud-send-shortcut"> · Ctrl/Cmd+Enter to send</span></>}</span></div>
        <div className="wa-ui-composer-input" data-uncertain={attempt.uncertain || undefined}>
          <textarea ref={composer} id={`${instance}-composer`} value={draft} onChange={(event) => { setDraft(event.target.value); if (!attempt.uncertain) setSendError(null); setSent(null) }} disabled={sending} aria-label={`Message to ${chat.title || identityLabel(chat.chat_id)}`} aria-describedby={`${instance}-shortcut`} className="wa-ui-cloud-textarea wa-ui-compose-editor" placeholder="Write a message…" rows={2} onKeyDown={(event) => {
            if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing && event.keyCode !== 229 && !event.repeat && connected && !sending && !requesting && !attempt.uncertain && !!draft.trim()) {
              event.preventDefault()
              void send()
            }
          }} />
          {attempt.uncertain && <Button type="button" onClick={archive.refresh} disabled={archive.loading || sending || requesting}><RefreshCw size={16} aria-hidden="true" />Check this conversation</Button>}
          {sendAction}
        </div>
      </div>
    </section>
    <LiveRegion announcement={announcement} />
  </>
}
