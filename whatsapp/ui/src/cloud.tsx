import { CloudComposer } from './cloud-composer'
import { Badge, Button, EmptyState, IconButton, List, ListItem, PageBody, PageMain, PageSidebar, SearchField, Skeleton, StatusPanel, type Host } from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useDebounce, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { ArrowDown, ArrowLeft, ChevronDown, Cloud, MessageCircle, MessageSquarePlus, RefreshCw } from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { type CloudConnection, type CloudSent, type CloudStatus } from './cloud-model'
import { mergeChats, mergeMessages, messageKey, messageLabel, type ChatTarget, type MessagePage, type ChatPage } from './inbox-model'
import { requestScope } from './model'
import { useInboxPages } from './use-inbox'
import { CloudTunnel } from './cloud-tunnel'
import { ChatIdentity, MessageRow } from './messages'

type Navigate = (action: () => void) => Promise<boolean>
const FIELD_LABELS: Record<string, string> = { business_account_id: 'business account ID', phone_number_id: 'phone number ID', access_token: 'access token', app_secret: 'app secret', verify_token: 'webhook verify token' }

export function CloudPanel({ host, narrow, panelSide, onDirty, navigate }: {
  host: Host; narrow: boolean; panelSide?: 'left' | 'right'; onDirty(value: boolean): void; navigate: Navigate
}) {
  const instance = useId()
  const status = useWorkerLive<CloudStatus>({ iii: host.iii, triggers: [], pollMs: 10000,
    handlerId: `iii::whatsapp-ui::cloud::${instance}`, fetch: () => host.iii.trigger('whatsapp::cloud::status', {}) })
  if (!status.data) return status.error
    ? <div className="wa-ui-reader-notice"><StatusPanel variant="alert" headline="Cloud API status unavailable" detail={status.error} action={<Button variant="ghost" onClick={status.refresh}>Try again</Button>} /></div>
    : <Skeleton className="wa-ui-loading" />
  const current = status.data
  const identity = `${current.enabled}:${current.business_account_id}:${current.phone_number_id}:${current.api_version}`
  return <CloudWorkspace key={identity} host={host} status={current} statusError={status.error} refreshStatus={status.refresh} narrow={narrow} panelSide={panelSide} onDirty={onDirty} navigate={navigate} />
}

function CloudWorkspace({ host, status, statusError, refreshStatus, narrow, panelSide, onDirty, navigate }: {
  host: Host; status: CloudStatus; statusError: string | null; refreshStatus(): void; narrow: boolean; panelSide?: 'left' | 'right'; onDirty(value: boolean): void; navigate: Navigate
}) {
  const [selected, setSelected] = useState<ChatTarget | null>(null)
  const [composing, setComposing] = useState(false)
  const [query, setQuery] = useState('')
  const search = useDebounce(query, 250)
  const [connection, setConnection] = useState<CloudConnection | null>(null)
  const [checking, setChecking] = useState(false)
  const [checkError, setCheckError] = useState<string | null>(null)
  const checkScope = useRef(requestScope())
  const checkBusy = useRef(false)
  const ready = status.enabled && status.configured && !statusError
  const needsSetup = !ready || !status.webhook_ready
  const [detailsOpen, setDetailsOpen] = useState(needsSetup)
  const detailsId = useId()
  useEffect(() => setDetailsOpen(needsSetup), [needsSetup])
  const chats = useInboxPages({ identity: search, enabled: status.enabled,
    fetchPage: async (cursor?: string) => {
      const page = await host.iii.trigger<ChatPage>('whatsapp::cloud::chats::list', { query: search || undefined, cursor, limit: 100 })
      return { ...page, items: page.chats }
    }, merge: mergeChats })
  useEffect(() => () => checkScope.current.invalidate(), [])
  const checkConnection = async () => {
    if (checkBusy.current || !ready) return
    checkBusy.current = true
    const current = checkScope.current.begin()
    setChecking(true); setCheckError(null); setConnection(null)
    try {
      const result = await host.iii.trigger<CloudConnection>('whatsapp::cloud::check_connection', {})
      if (current()) setConnection(result)
    } catch (cause) { if (current()) setCheckError(errorMessage(cause)) }
    finally { if (current()) { checkBusy.current = false; setChecking(false) } }
  }
  const openChat = (chat: ChatTarget | null) => {
    if (composing && selected?.chat_id === chat?.chat_id) return
    void navigate(() => { setSelected(chat); setComposing(true) })
  }
  const leaveChat = () => { void navigate(() => { setSelected(null); setComposing(false) }) }
  const messageSent = (sent: CloudSent) => {
    chats.refresh()
    if (!selected) setSelected({ chat_id: sent.chat_id, title: sent.chat_id, is_group: false })
  }
  const missing = status.missing_fields.map((field) => FIELD_LABELS[field.replace(/^cloud\./, '')] ?? field).join(', ')
  return <div className="wa-ui-cloud">
    <section className="wa-ui-cloud-status" aria-label="Cloud API connection">
      <div className="wa-ui-cloud-heading"><h2 className="wa-ui-title">Meta Cloud API</h2><Badge variant={needsSetup ? 'warn' : 'ok'}>{statusError ? 'Status unavailable' : !status.enabled ? 'Disabled' : !status.configured ? 'Setup needed' : !status.webhook_ready ? 'Webhook setup needed' : 'Configured'}</Badge><Button className="wa-ui-cloud-details-toggle" variant="ghost" aria-label="Connection details" title="Connection details" aria-expanded={detailsOpen} aria-controls={detailsId} onClick={() => setDetailsOpen((value) => !value)}><span>{narrow ? 'Details' : 'Connection details'}</span><ChevronDown size={16} aria-hidden="true" /></Button></div>
      <div id={detailsId} className="wa-ui-cloud-details" hidden={!detailsOpen}><div className="wa-ui-stack">
      <div className="wa-ui-cloud-heading"><p className="wa-ui-note">Official WhatsApp Business Platform · separate from linked phone sessions.</p><Button variant="ghost" disabled={!ready || checking} onClick={() => void checkConnection()}><RefreshCw size={16} aria-hidden="true" />{checking ? 'Checking…' : 'Check connection'}</Button></div>
      {statusError && <StatusPanel variant="warn" headline="Could not refresh Cloud API status" detail={statusError} action={<Button variant="ghost" onClick={refreshStatus}>Try again</Button>} />}
      {!ready && !statusError && <p className="wa-ui-connection-copy">Use Configure worker in the page header to {status.enabled ? 'complete setup' : 'enable the Cloud API'}.{missing ? ` Add your ${missing}.` : ''} Your native sessions remain available in the other tabs.</p>}
      {checkError && <StatusPanel variant="alert" headline="Connection check failed" detail={checkError} />}
      {connection && <StatusPanel className="wa-ui-cloud-result" variant={connection.phone_in_business_account ? 'success' : 'warn'} headline={connection.phone_in_business_account ? 'Business number verified' : 'Phone number not found in this business account'} detail={[connection.phone.verified_name, connection.phone.display_phone_number, connection.phone.quality_rating && `Quality: ${connection.phone.quality_rating}`, `${connection.subscribed_apps.length} subscribed ${connection.subscribed_apps.length === 1 ? 'app' : 'apps'}`].filter(Boolean).join(' · ')} />}
      <details className="wa-ui-cloud-setup"><summary>Account and webhook details<ChevronDown size={16} aria-hidden="true" /></summary><div className="wa-ui-stack">
        <p className="wa-ui-note">Business account: <span className="wa-ui-mono">{status.business_account_id || 'Not set'}</span> · Phone number ID: <span className="wa-ui-mono">{status.phone_number_id || 'Not set'}</span> · {status.api_version}</p>
        <p className="wa-ui-note">{status.webhook_ready ? 'Webhook verification credentials are set.' : 'Set the app secret and verify token to receive signed webhooks.'} Configure the public callback URL in Meta with this path: <span className="wa-ui-mono">{status.webhook_path}</span>.</p>
        <p className="wa-ui-note">Only messages captured by this worker appear below. Earlier phone history and native phone groups are not imported.</p>
      </div></details>
      <CloudTunnel host={host} webhookReady={status.enabled && status.webhook_ready && !statusError} />
      </div></div>
    </section>
    <PageBody className="wa-ui-browser" side={panelSide}>
      {(!narrow || !composing) && <PageSidebar label="Cloud conversations" side={panelSide} narrow={narrow} defaultWidth={320} minWidth={240} maxWidth={400} header={<div className="wa-ui-sidebar-heading"><span>Conversations</span><div className="wa-ui-header-actions"><IconButton label="New Cloud message" onClick={() => openChat(null)}><MessageSquarePlus size={16} /></IconButton><IconButton label="Refresh Cloud conversations" onClick={chats.refresh} disabled={chats.loading}><RefreshCw size={16} /></IconButton></div></div>}>
        <div className="wa-ui-list-tools"><SearchField aria-label="Search Cloud conversations" placeholder="Search conversations" value={query} onChange={setQuery} /><span className="wa-ui-note">{chats.items.length.toLocaleString('en-US')} {chats.items.length === 1 ? 'conversation' : 'conversations'}</span></div>
        {chats.error && <div className="wa-ui-reader-notice"><StatusPanel variant="alert" headline="Could not load Cloud conversations" detail={chats.error} action={<Button variant="ghost" onClick={chats.refresh}>Try again</Button>} /></div>}
        <div className="wa-ui-list">{chats.loading ? <Skeleton className="wa-ui-loading" /> : chats.items.length ? <List aria-label="Cloud conversations">{chats.items.map((chat) => <ListItem key={chat.chat_id} className="wa-ui-chat-item" selected={selected?.chat_id === chat.chat_id} onClick={() => openChat(chat)} leading={<ChatIdentity chat={chat} />} label={<span className="wa-ui-chat-heading"><span className="wa-ui-chat-title" title={chat.title || chat.chat_id}>{chat.title || chat.chat_id}</span>{chat.last_message && <time className="wa-ui-chat-meta" dateTime={new Date(chat.last_message.timestamp_ms).toISOString()}>{formatRelative(chat.last_message.timestamp_ms)}</time>}</span>} description={<span className="wa-ui-preview">{chat.last_message ? `${chat.last_message.from_me ? 'You: ' : ''}${messageLabel(chat.last_message)}` : 'No message preview'}</span>} />)}</List> : <EmptyState compact icon={Cloud} title={!status.enabled ? 'Cloud API is disabled' : query ? 'No conversations found' : 'No Cloud messages yet'} description={!status.enabled ? 'Enable it in Configure worker to use the business message archive.' : query ? 'Try another name or identifier.' : 'Received webhooks and accepted outgoing messages appear here.'} />}
          {chats.nextCursor && <div className="wa-ui-list-tools"><Button variant="ghost" onClick={chats.more} disabled={chats.loadingMore}>{chats.loadingMore ? 'Loading…' : 'Load more conversations'}</Button></div>}
        </div><p className="wa-ui-cloud-archive-note wa-ui-note">Local Cloud API archive{chats.coverage?.retention_truncated ? ` · Older messages removed at the ${chats.coverage.message_limit.toLocaleString('en-US')}-message limit.` : ''}</p>
      </PageSidebar>}
      {(!narrow || composing) && <PageMain className="wa-ui-reader">{composing
        ? <CloudConversation key={selected?.chat_id ?? 'new'} host={host} chat={selected} ready={ready} narrow={narrow} onBack={leaveChat} onDirty={onDirty} onSent={messageSent} />
        : <EmptyState icon={Cloud} title="Cloud API conversations" description="Select a conversation or write a new message from your business number." action={{ label: 'New message', onClick: () => openChat(null) }} />}</PageMain>}
    </PageBody>
  </div>
}

const dayLabel = (timestamp: number) => new Date(timestamp).toLocaleDateString('en-US', { day: 'numeric', month: 'long', year: 'numeric' })

function CloudConversation({ host, chat, ready, narrow, onBack, onDirty, onSent }: {
  host: Host; chat: ChatTarget | null; ready: boolean; narrow: boolean; onBack(): void
  onDirty(value: boolean): void; onSent(result: CloudSent): void
}) {
  const archive = useInboxPages({ identity: chat?.chat_id ?? '', enabled: !!chat,
    fetchPage: async (cursor?: string) => { const page = await host.iii.trigger<MessagePage>('whatsapp::cloud::messages::list', { chat_id: chat!.chat_id, cursor, limit: 50 }); return { ...page, items: page.messages } },
    merge: (previous, incoming) => mergeMessages(previous, incoming, chat!.chat_id) })
  const scroll = useRef<HTMLDivElement>(null)
  const followLatest = useRef(true)
  const anchor = useRef<{ key: string; offset: number } | null>(null)
  const size = useRef<{ width: number; height: number } | null>(null)
  const [showLatest, setShowLatest] = useState(false)
  const rememberPosition = () => {
    const node = scroll.current
    if (!node || (size.current && (node.clientWidth !== size.current.width || node.clientHeight !== size.current.height))) return
    followLatest.current = node.scrollHeight - node.scrollTop - node.clientHeight < 96
    setShowLatest(!followLatest.current)
    const top = node.getBoundingClientRect().top
    const first = Array.from(node.querySelectorAll<HTMLElement>('.wa-ui-message-entry')).find((entry) => entry.querySelector('.wa-ui-message-row')!.getBoundingClientRect().bottom > top)
    const row = first?.querySelector('.wa-ui-message-row')
    anchor.current = first && row ? { key: first.dataset.messageKey!, offset: row.getBoundingClientRect().top - top } : null
  }
  const restorePosition = () => {
    const node = scroll.current
    if (!node) return
    if (followLatest.current) node.scrollTop = node.scrollHeight
    else if (anchor.current) {
      const entry = Array.from(node.querySelectorAll<HTMLElement>('.wa-ui-message-entry')).find((element) => element.dataset.messageKey === anchor.current!.key)
      const row = entry?.querySelector('.wa-ui-message-row')
      if (row) node.scrollTop += row.getBoundingClientRect().top - node.getBoundingClientRect().top - anchor.current.offset
    }
  }
  const jumpToLatest = () => {
    followLatest.current = true
    anchor.current = null
    setShowLatest(false)
    restorePosition()
  }
  useLayoutEffect(restorePosition, [archive.items, archive.loadingMore])
  useEffect(() => {
    const node = scroll.current
    if (!node) return
    size.current = { width: node.clientWidth, height: node.clientHeight }
    const resize = new ResizeObserver(() => {
      size.current = { width: node.clientWidth, height: node.clientHeight }
      restorePosition()
    })
    resize.observe(node)
    return () => resize.disconnect()
  }, [])
  const latestIncomingAt = archive.items.reduce<number | undefined>((latest, message) => message.from_me ? latest : Math.max(latest ?? 0, message.timestamp_ms), undefined)
  const sent = (result: CloudSent) => { jumpToLatest(); archive.refresh(); onSent(result) }
  return <>
    <header className="wa-ui-detail-header wa-ui-compact-header">
      {narrow && <IconButton label="Back to Cloud conversations" onClick={onBack}><ArrowLeft size={16} /></IconButton>}
      {chat && <ChatIdentity chat={chat} header />}
      <div className="wa-ui-heading-copy"><h3 className="wa-ui-title">{chat?.title || chat?.chat_id || 'New Cloud message'}</h3><p className="wa-ui-note wa-ui-identity">{chat ? chat.chat_id : 'Choose a recipient and message type.'}</p></div>
      {chat && <IconButton label="Refresh Cloud messages" onClick={archive.refresh} disabled={archive.loading}><RefreshCw size={16} /></IconButton>}
    </header>
    <div className="wa-ui-cloud-reader-body" data-new={!chat || undefined}>
      {chat && <div className="wa-ui-cloud-history">
        <div ref={scroll} className="wa-ui-cloud-messages" aria-label={`Stored Cloud messages with ${chat.title || chat.chat_id}`} onScroll={rememberPosition}>
          {archive.error ? <StatusPanel variant="alert" headline="Could not load Cloud messages" detail={archive.error} action={<Button variant="ghost" onClick={archive.refresh}>Try again</Button>} /> : archive.loading ? <Skeleton className="wa-ui-loading" /> : !archive.items.length ? <EmptyState compact icon={MessageCircle} title="No stored messages" description="This worker has not captured messages for this recipient yet." /> : <>
            {archive.nextCursor && <div className="wa-ui-load-older"><Button variant="ghost" onClick={() => { rememberPosition(); followLatest.current = false; setShowLatest(true); archive.more() }} disabled={archive.loadingMore}>{archive.loadingMore ? 'Loading…' : 'Load older stored messages'}</Button></div>}
            <div className="wa-ui-message-thread" role="log" aria-label="Cloud message history" aria-live="off">{archive.items.map((message, index) => <div key={messageKey(message)} data-message-key={messageKey(message)} className="wa-ui-message-entry">
              {(index === 0 || dayLabel(archive.items[index - 1].timestamp_ms) !== dayLabel(message.timestamp_ms)) && <div className="wa-ui-message-date">{dayLabel(message.timestamp_ms)}</div>}
              <MessageRow message={message} group={false} />
            </div>)}</div>
          </>}
        </div>
        {showLatest && <div className="wa-ui-cloud-jump"><Button onClick={jumpToLatest}><ArrowDown size={16} aria-hidden="true" />Jump to latest</Button></div>}
      </div>}
      <CloudComposer host={host} chat={chat} ready={ready} latestIncomingAt={latestIncomingAt} onDirty={onDirty} onSent={sent} />
    </div>
  </>
}
