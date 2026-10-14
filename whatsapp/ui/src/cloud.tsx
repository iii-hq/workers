import { CloudComposer, type CloudMessageMode } from './cloud-composer'
import { Badge, Button, EmptyState, IconButton, List, ListItem, PageBody, PageMain, PageSidebar, SearchField, Skeleton, StatusPanel, WorkerConfigurationDialog, type Host } from '@iii-dev/console-ui'
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
import { TestMessage } from './test-message'
import { DraftScope, useDraftScope, usePageState } from './drafts'

type Navigate = (action: () => void) => Promise<boolean>
const FIELD_LABELS: Record<string, string> = { business_account_id: 'business account ID', phone_number_id: 'phone number ID', access_token: 'access token', app_secret: 'app secret', verify_token: 'webhook verify token' }

export function CloudPanel({ host, narrow, panelSide, onDirty, navigate, configurationId = 'whatsapp' }: {
  host: Host; narrow: boolean; panelSide?: 'left' | 'right'; onDirty(value: boolean): void; navigate: Navigate; configurationId?: string
}) {
  const instance = useId()
  const [archiveIdentity, setArchiveIdentity] = useState<string | null>(null)
  const status = useWorkerLive<CloudStatus>({ iii: host.iii, triggers: [], pollMs: 10000,
    handlerId: `iii::whatsapp-ui::cloud::${instance}`, fetch: () => host.iii.trigger('whatsapp::cloud::status', {}) })
  if (!status.data) return status.error
    ? <div className="wa-ui-reader-notice"><StatusPanel role="alert" variant="alert" headline="Cloud API status unavailable" detail={status.error} action={<Button variant="ghost" onClick={status.refresh}>Try again</Button>} /></div>
    : <Skeleton className="wa-ui-loading" />
  const current = status.data
  const identity = `${current.enabled}:${current.business_account_id}:${current.phone_number_id}:${current.api_version}`
  const needsAccount = !current.enabled || !current.configured
  if (needsAccount && archiveIdentity !== identity) return <CloudSetup host={host} configurationId={configurationId} status={current} statusError={status.error} refreshStatus={status.refresh} onBrowse={() => setArchiveIdentity(identity)} />
  return <DraftScope id={`cloud:${current.business_account_id}:${current.phone_number_id}`}><CloudWorkspace key={identity} host={host} status={current} statusError={status.error} refreshStatus={status.refresh} narrow={narrow} panelSide={panelSide} onDirty={onDirty} navigate={navigate} initialHistoryOpen={archiveIdentity === identity} onSetup={needsAccount ? () => { void navigate(() => setArchiveIdentity(null)) } : undefined} /></DraftScope>
}

function CloudSetup({ host, status, statusError, refreshStatus, onBrowse, configurationId }: { host: Host; status: CloudStatus; statusError: string | null; refreshStatus(): void; onBrowse(): void; configurationId: string }) {
  const headingId = useId()
  const toolsId = useId()
  const [toolsOpen, setToolsOpen] = useState(false)
  const [configureOpen, setConfigureOpen] = useState(false)
  const hasAccount = status.enabled && !!status.phone_number_id.trim()
  const missing = status.missing_fields.map((field) => FIELD_LABELS[field.replace(/^cloud\./, '')] ?? field).join(', ')
  return <div className="wa-ui-onboarding-layout"><section className="wa-ui-onboarding" aria-labelledby={headingId}>
    <h2 id={headingId} className="wa-ui-onboarding-title">Set up Meta Cloud API</h2>
    <p className="wa-ui-onboarding-copy">Use your WhatsApp Business Platform account to test messages and webhooks. No linked phone session is needed.</p>
    {statusError && <StatusPanel role="status" variant="warn" headline="Could not refresh Cloud API status" detail={statusError} action={<Button variant="ghost" onClick={refreshStatus}>Try again</Button>} />}
    <div className="wa-ui-actions"><Button onClick={() => setConfigureOpen(true)}>Configure Cloud API</Button></div>
    {missing && <p className="wa-ui-note">Still needed: {missing}.</p>}
    <WorkerConfigurationDialog configurationId={configureOpen ? configurationId : null} onClose={() => setConfigureOpen(false)} />
    <ol className="wa-ui-setup-steps">
      <li><strong>Account</strong><p>Enable Cloud API and add your business account ID, phone number ID, and access token.</p></li>
      <li><strong>Webhook</strong><p>Add your app secret and webhook verify token, then register the callback URL in Meta.</p></li>
      <li><strong>Test</strong><p>Check the connection, send to a phone you control, and reply to verify incoming messages.</p></li>
    </ol>
    {hasAccount && <>
      <div className="wa-ui-actions"><Button variant="ghost" onClick={onBrowse}>Browse stored conversations</Button><Button variant="ghost" aria-expanded={toolsOpen} aria-controls={toolsId} onClick={() => setToolsOpen((open) => !open)}>Webhook tools<ChevronDown size={16} aria-hidden="true" /></Button></div>
      <div id={toolsId} hidden={!toolsOpen}>{toolsOpen && <CloudTunnel host={host} webhookReady={status.webhook_ready && !statusError} />}</div>
    </>}
    <p className="wa-ui-note">To connect using a QR code and browse your phone’s messages and groups, choose Linked phone.</p>
  </section></div>
}

function CloudWorkspace({ host, status, statusError, refreshStatus, narrow, panelSide, onDirty, navigate, initialHistoryOpen = false, onSetup }: {
  host: Host; status: CloudStatus; statusError: string | null; refreshStatus(): void; narrow: boolean; panelSide?: 'left' | 'right'; onDirty(value: boolean): void; navigate: Navigate
  initialHistoryOpen?: boolean; onSetup?(): void
}) {
  const workspaceKey = `${useDraftScope()}:workspace`
  const [selected, setSelected] = usePageState<ChatTarget | null>(`${workspaceKey}:recipient`, null)
  const [composing, setComposing] = usePageState(`${workspaceKey}:composing`, false)
  const [messageMode, setMessageMode] = usePageState<CloudMessageMode>(`${workspaceKey}:mode`, 'text')
  const [lastSent, setLastSent] = usePageState<CloudSent | null>(`${workspaceKey}:receipt`, null)
  const [historyOpen, setHistoryOpen] = useState(initialHistoryOpen)
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
  const preparationId = useId()
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
  const openChat = (chat: ChatTarget | null, mode: CloudMessageMode = 'text') => {
    if (composing && selected?.chat_id === chat?.chat_id && messageMode === mode) return
    void navigate(() => { setSelected(chat); setMessageMode(mode); setComposing(true) })
  }
  const leaveChat = () => { void navigate(() => { setSelected(null); setComposing(false); setHistoryOpen(!ready) }) }
  const messageSent = (sent: CloudSent, mode: CloudMessageMode) => {
    setLastSent(sent)
    setMessageMode(mode)
    chats.refresh()
    if (!selected) setSelected({ chat_id: sent.chat_id, title: sent.chat_id, is_group: false })
  }
  return <div className="wa-ui-cloud">
    <section className="wa-ui-cloud-status" aria-label="Cloud API connection">
      <div className="wa-ui-cloud-heading"><h2 className="wa-ui-title">Meta Cloud API</h2><Badge variant={needsSetup ? 'warn' : 'ok'}>{statusError ? 'Status unavailable' : !status.enabled ? 'Disabled' : !status.configured ? 'Setup needed' : !status.webhook_ready ? 'Webhook setup needed' : 'Configured'}</Badge>{onSetup ? <Button variant="ghost" onClick={onSetup}>Back to setup</Button> : <Button variant="ghost" disabled={!ready || checking} onClick={() => void checkConnection()}><RefreshCw size={16} aria-hidden="true" />{checking ? 'Checking…' : 'Check connection'}</Button>}<Button className="wa-ui-cloud-details-toggle" variant="ghost" aria-label="Webhook setup" title="Webhook setup" aria-expanded={detailsOpen} aria-controls={detailsId} onClick={() => setDetailsOpen((value) => !value)}><span>Webhook setup</span><ChevronDown size={16} aria-hidden="true" /></Button></div>
      {checkError && <StatusPanel role="alert" variant="alert" headline="Connection check failed" detail={checkError} />}
      {connection && <StatusPanel role="status" className="wa-ui-cloud-result" variant={connection.phone_in_business_account ? 'success' : 'warn'} headline={connection.phone_in_business_account ? 'Business number verified' : 'Phone number not found in this business account'} detail={[connection.phone.verified_name, connection.phone.display_phone_number, connection.phone.quality_rating && `Quality: ${connection.phone.quality_rating}`, `${connection.subscribed_apps.length} subscribed ${connection.subscribed_apps.length === 1 ? 'app' : 'apps'}`].filter(Boolean).join(' · ')} />}
      <div id={detailsId} className="wa-ui-cloud-details" hidden={!detailsOpen}><div className="wa-ui-stack">
      <p className="wa-ui-note">Official WhatsApp Business Platform · separate from linked phone sessions.</p>
      {statusError && <StatusPanel role="status" variant="warn" headline="Could not refresh Cloud API status" detail={statusError} action={<Button variant="ghost" onClick={refreshStatus}>Try again</Button>} />}
      <details className="wa-ui-cloud-setup"><summary>Account and webhook details<ChevronDown size={16} aria-hidden="true" /></summary><div className="wa-ui-stack">
        <p className="wa-ui-note">Business account: <span className={status.business_account_id ? 'wa-ui-mono' : undefined}>{status.business_account_id || 'Not set'}</span> · Phone number ID: <span className={status.phone_number_id ? 'wa-ui-mono' : undefined}>{status.phone_number_id || 'Not set'}</span> · <span className="wa-ui-mono">{status.api_version}</span></p>
        <p className="wa-ui-note">{status.webhook_ready ? 'Webhook verification credentials are set.' : 'Set the app secret and verify token to receive signed webhooks.'} Configure the public callback URL in Meta with this path: <span className="wa-ui-mono">{status.webhook_path}</span>.</p>
        <p className="wa-ui-note">Only messages captured by this worker appear below. Earlier phone history and native phone groups are not imported.</p>
      </div></details>
      <CloudTunnel host={host} webhookReady={status.enabled && status.webhook_ready && !statusError} />
      </div></div>
    </section>
    <PageBody className="wa-ui-browser" side={panelSide}>
      {(!narrow || (!composing && historyOpen)) && <PageSidebar label="Cloud conversations" side={panelSide} narrow={narrow} defaultWidth={288} minWidth={240} maxWidth={400} header={<div className="wa-ui-sidebar-heading">{narrow && ready && <IconButton label="Back to Cloud tests" onClick={leaveChat}><ArrowLeft size={16} /></IconButton>}<span>Stored conversations</span><div className="wa-ui-header-actions">{ready && <IconButton label="New Cloud message" onClick={() => openChat(null)}><MessageSquarePlus size={16} /></IconButton>}<IconButton label="Refresh Cloud conversations" onClick={chats.refresh} disabled={chats.loading}><RefreshCw size={16} /></IconButton></div></div>}>
        <div className="wa-ui-list-tools"><SearchField aria-label="Search Cloud conversations" placeholder="Search conversations" value={query} onChange={setQuery} /><span className="wa-ui-note">{chats.items.length.toLocaleString('en-US')} {chats.items.length === 1 ? 'conversation' : 'conversations'}</span></div>
        {chats.error && <div className="wa-ui-reader-notice"><StatusPanel role="alert" variant="alert" headline="Could not load Cloud conversations" detail={chats.error} action={<Button variant="ghost" onClick={chats.refresh}>Try again</Button>} /></div>}
        <div className="wa-ui-list">{chats.loading ? <Skeleton className="wa-ui-loading" /> : chats.items.length ? <List aria-label="Cloud conversations">{chats.items.map((chat) => <ListItem key={chat.chat_id} className="wa-ui-chat-item" selected={selected?.chat_id === chat.chat_id} onClick={() => openChat(chat)} leading={<ChatIdentity chat={chat} />} label={<span className="wa-ui-chat-heading"><span className={`wa-ui-chat-title${!chat.title || chat.title === chat.chat_id || /^\+?[\d\s()-]+$/.test(chat.title) ? ' wa-ui-mono' : ''}`} title={chat.title || chat.chat_id}>{chat.title || chat.chat_id}</span>{chat.last_message && <time className="wa-ui-chat-meta" dateTime={new Date(chat.last_message.timestamp_ms).toISOString()}>{formatRelative(chat.last_message.timestamp_ms)}</time>}</span>} description={<span className="wa-ui-preview">{chat.last_message ? `${chat.last_message.from_me ? 'You: ' : ''}${messageLabel(chat.last_message)}` : 'No message preview'}</span>} />)}</List> : <EmptyState compact icon={Cloud} title={!status.enabled ? 'Cloud API is disabled' : query ? 'No conversations found' : 'No Cloud messages yet'} description={!status.enabled ? 'Enable it in Configure worker to use the business message archive.' : query ? 'Try another name or identifier.' : 'Received webhooks and accepted outgoing messages appear here.'} />}
          {chats.nextCursor && <div className="wa-ui-list-tools"><Button variant="ghost" onClick={chats.more} disabled={chats.loadingMore}>{chats.loadingMore ? 'Loading…' : 'Load more conversations'}</Button></div>}
        </div><p className="wa-ui-cloud-archive-note wa-ui-note">Local Cloud API archive{chats.coverage?.retention_truncated ? ` · Older messages removed at the ${chats.coverage.message_limit.toLocaleString('en-US')}-message limit.` : ''}</p>
      </PageSidebar>}
      {(!narrow || composing || !historyOpen) && <PageMain className="wa-ui-reader">{composing
        ? <CloudConversation key={`${selected?.chat_id ?? 'new'}:${messageMode}`} host={host} chat={selected} initialMode={messageMode} initialSent={lastSent && lastSent.chat_id === selected?.chat_id ? lastSent : undefined} readOnly={!!onSetup} ready={ready} narrow={narrow} onBack={leaveChat} onDirty={onDirty} onSent={messageSent} onCheckConversation={openChat} />
        : !onSetup ? <div className="wa-ui-test-landing"><section className="wa-ui-test-preparation" aria-labelledby={preparationId}>
          <h2 id={preparationId} className="wa-ui-title">Start a Cloud test</h2>
          <div className="wa-ui-test-paths">
            <div><h3 className="wa-ui-title">Receive a message first</h3><p className="wa-ui-note">From a phone you control, message {connection?.phone.display_phone_number ? <strong className="wa-ui-mono">{connection.phone.display_phone_number}</strong> : 'your business number'} to open the 24-hour text reply window.</p>{!connection?.phone.display_phone_number && <p className="wa-ui-note">Check connection above to display your number.</p>}</div>
            <div><Button variant="ghost" onClick={() => openChat(null, 'template')} disabled={!ready}>Use an approved template</Button><p className="wa-ui-note">Start without a recent incoming message.</p></div>
          </div>
        </section><TestMessage host={host} transport="cloud" ready={ready} onDirty={onDirty} onOpenConversation={openChat} /><details className="wa-ui-test-help"><summary>How to verify the test<ChevronDown size={16} aria-hidden="true" /></summary><p className="wa-ui-note">After sending, reply from the test phone to verify receiving. Delivery is confirmed separately by the webhook. Outside the 24-hour reply window, use an approved template. <a href="https://whatsappbusiness.com/policy/" target="_blank" rel="noopener noreferrer">Messaging rules</a></p></details><div className="wa-ui-test-links"><Button variant="ghost" onClick={() => openChat(null)}>Test templates or attachments</Button>{narrow && <Button variant="ghost" onClick={() => void navigate(() => setHistoryOpen(true))}>Browse stored conversations</Button>}</div></div>
        : <EmptyState icon={MessageCircle} title="Select a stored conversation" description="Stored messages remain available. Complete account setup in Configure worker to send messages." />}</PageMain>}
    </PageBody>
  </div>
}

const dayLabel = (timestamp: number) => new Date(timestamp).toLocaleDateString('en-US', { day: 'numeric', month: 'long', year: 'numeric' })

function CloudConversation({ host, chat, readOnly, ready, narrow, onBack, onDirty, onSent, onCheckConversation, initialMode, initialSent }: {
  host: Host; chat: ChatTarget | null; readOnly: boolean; ready: boolean; narrow: boolean; onBack(): void
  onDirty(value: boolean): void; onSent(result: CloudSent, mode: CloudMessageMode): void; onCheckConversation(target: ChatTarget): void; initialMode?: CloudMessageMode; initialSent?: CloudSent
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
  const sent = (result: CloudSent, mode: CloudMessageMode) => { jumpToLatest(); archive.refresh(); onSent(result, mode) }
  const checkConversation = (target: ChatTarget) => { if (chat?.chat_id === target.chat_id) archive.refresh(); else onCheckConversation(target) }
  return <>
    <header className="wa-ui-detail-header wa-ui-compact-header">
      <IconButton label={ready ? 'Back to Cloud tests' : 'Back to stored conversations'} onClick={onBack}><ArrowLeft size={16} /></IconButton>
      {chat && <ChatIdentity chat={chat} header />}
      <div className="wa-ui-heading-copy"><h3 className={`wa-ui-title${chat && (!chat.title || chat.title === chat.chat_id || /^\+?[\d\s()-]+$/.test(chat.title)) ? ' wa-ui-mono' : ''}`}>{chat?.title || chat?.chat_id || 'New Cloud message'}</h3>{(!chat || (chat.title && chat.title !== chat.chat_id)) && <p className={`wa-ui-note wa-ui-identity${chat ? ' wa-ui-mono' : ''}`}>{chat ? chat.chat_id : 'Choose a recipient and message type.'}</p>}</div>
      {chat && <IconButton label="Refresh Cloud messages" onClick={archive.refresh} disabled={archive.loading}><RefreshCw size={16} /></IconButton>}
    </header>
    <div className="wa-ui-cloud-reader-body" data-new={!chat || undefined}>
      {chat && <div className="wa-ui-cloud-history">
        <div ref={scroll} className="wa-ui-cloud-messages" aria-label={`Stored Cloud messages with ${chat.title || chat.chat_id}`} onScroll={rememberPosition}>
          {archive.error ? <StatusPanel role="alert" variant="alert" headline="Could not load Cloud messages" detail={archive.error} action={<Button variant="ghost" onClick={archive.refresh}>Try again</Button>} /> : archive.loading ? <Skeleton className="wa-ui-loading" /> : !archive.items.length ? <EmptyState compact icon={MessageCircle} title="No stored messages" description="This worker has not captured messages for this recipient yet." /> : <>
            {archive.nextCursor && <div className="wa-ui-load-older"><Button variant="ghost" onClick={() => { rememberPosition(); followLatest.current = false; setShowLatest(true); archive.more() }} disabled={archive.loadingMore}>{archive.loadingMore ? 'Loading…' : 'Load older stored messages'}</Button></div>}
            <div className="wa-ui-message-thread" role="log" aria-label="Cloud message history" aria-live="off">{archive.items.map((message, index) => <div key={messageKey(message)} data-message-key={messageKey(message)} className="wa-ui-message-entry">
              {(index === 0 || dayLabel(archive.items[index - 1].timestamp_ms) !== dayLabel(message.timestamp_ms)) && <div className="wa-ui-message-date">{dayLabel(message.timestamp_ms)}</div>}
              <MessageRow message={message} group={chat.is_group || chat.chat_id.startsWith('group:')} />
            </div>)}</div>
          </>}
        </div>
        {showLatest && <div className="wa-ui-cloud-jump"><Button onClick={jumpToLatest}><ArrowDown size={16} aria-hidden="true" />Jump to latest</Button></div>}
      </div>}
      {!readOnly ? <CloudComposer host={host} chat={chat} ready={ready} initialMode={initialMode} initialSent={initialSent} latestIncomingAt={latestIncomingAt} onDirty={onDirty} onSent={sent} onCheckConversation={checkConversation} />
        : <div className="wa-ui-reader-notice"><p className="wa-ui-note">Read only · Complete account setup in Configure worker to send messages.</p></div>}
    </div>
  </>
}
