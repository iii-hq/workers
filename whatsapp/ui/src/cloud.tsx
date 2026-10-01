import { Badge, Button, CodeEditor, EmptyState, IconButton, Input, List, ListItem, PageBody, PageMain, PageSidebar, SearchField, Select, SettingsField, SettingsList, Skeleton, StatusPanel, type Host } from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useDebounce, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { ArrowLeft, Check, ChevronDown, Cloud, MessageCircle, MessageSquarePlus, RefreshCw, Send } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import { cloudRecipient, cloudTextError, templateComponents, templateText, type CloudConnection, type CloudSent, type CloudStatus, type CloudTemplate, type TemplatePage } from './cloud-model'
import { mergeChats, mergeMessages, messageKey, messageLabel, messageStatus, type ChatTarget, type MessagePage, type ChatPage } from './inbox-model'
import { requestScope } from './model'
import { useInboxPages } from './use-inbox'
import { CloudTunnel } from './cloud-tunnel'

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
  const missing = status.missing_fields.map((field) => FIELD_LABELS[field.replace(/^cloud\./, '')] ?? field).join(', ')
  return <div className="wa-ui-cloud">
    <section className="wa-ui-cloud-status" aria-label="Cloud API connection">
      <div className="wa-ui-cloud-heading"><div className="wa-ui-heading-copy"><h2 className="wa-ui-title">Meta Cloud API</h2><p className="wa-ui-note">Official WhatsApp Business Platform · separate from linked phone sessions.</p></div><Badge variant={ready ? 'ok' : 'warn'}>{!status.enabled ? 'Disabled' : status.configured ? 'Configured' : 'Setup needed'}</Badge><Button variant="ghost" disabled={!ready || checking} onClick={() => void checkConnection()}><RefreshCw size={16} aria-hidden="true" />{checking ? 'Checking…' : 'Check connection'}</Button></div>
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
    </section>
    <PageBody className="wa-ui-browser" side={panelSide}>
      {(!narrow || !composing) && <PageSidebar label="Cloud conversations" side={panelSide} narrow={narrow} defaultWidth={320} minWidth={240} maxWidth={400} header={<div className="wa-ui-sidebar-heading"><span>Conversations</span><div className="wa-ui-header-actions"><IconButton label="New Cloud message" onClick={() => openChat(null)}><MessageSquarePlus size={16} /></IconButton><IconButton label="Refresh Cloud conversations" onClick={chats.refresh} disabled={chats.loading}><RefreshCw size={16} /></IconButton></div></div>}>
        <div className="wa-ui-list-tools"><SearchField aria-label="Search Cloud conversations" placeholder="Search conversations" value={query} onChange={setQuery} /><span className="wa-ui-note">{chats.items.length.toLocaleString('en-US')} {chats.items.length === 1 ? 'conversation' : 'conversations'}</span></div>
        {chats.error && <div className="wa-ui-reader-notice"><StatusPanel variant="alert" headline="Could not load Cloud conversations" detail={chats.error} action={<Button variant="ghost" onClick={chats.refresh}>Try again</Button>} /></div>}
        <div className="wa-ui-list">{chats.loading ? <Skeleton className="wa-ui-loading" /> : chats.items.length ? <List aria-label="Cloud conversations">{chats.items.map((chat) => <ListItem key={chat.chat_id} className="wa-ui-chat-item" selected={selected?.chat_id === chat.chat_id} onClick={() => openChat(chat)} leading={<span className="wa-ui-avatar" aria-hidden="true"><MessageCircle size={16} /></span>} label={<span className="wa-ui-chat-heading"><span className="wa-ui-chat-title" title={chat.title || chat.chat_id}>{chat.title || chat.chat_id}</span>{chat.last_message && <time className="wa-ui-chat-meta" dateTime={new Date(chat.last_message.timestamp_ms).toISOString()}>{formatRelative(chat.last_message.timestamp_ms)}</time>}</span>} description={<span className="wa-ui-preview">{chat.last_message ? `${chat.last_message.from_me ? 'You: ' : ''}${messageLabel(chat.last_message)}` : 'No message preview'}</span>} />)}</List> : <EmptyState compact icon={Cloud} title={!status.enabled ? 'Cloud API is disabled' : query ? 'No conversations found' : 'No Cloud messages yet'} description={!status.enabled ? 'Enable it in Configure worker to use the business message archive.' : query ? 'Try another name or identifier.' : 'Received webhooks and accepted outgoing messages appear here.'} />}
          {chats.nextCursor && <div className="wa-ui-list-tools"><Button variant="ghost" onClick={chats.more} disabled={chats.loadingMore}>{chats.loadingMore ? 'Loading…' : 'Load more conversations'}</Button></div>}
        </div><p className="wa-ui-cloud-archive-note wa-ui-note">Local Cloud API archive{chats.coverage?.retention_truncated ? ` · Older messages removed at the ${chats.coverage.message_limit.toLocaleString('en-US')}-message limit.` : ''}</p>
      </PageSidebar>}
      {(!narrow || composing) && <PageMain className="wa-ui-reader">{composing ? <>
        <header className="wa-ui-detail-header wa-ui-compact-header">{narrow && <IconButton label="Back to Cloud conversations" onClick={leaveChat}><ArrowLeft size={16} /></IconButton>}<div className="wa-ui-heading-copy"><h3 className="wa-ui-title">{selected?.title || selected?.chat_id || 'New Cloud message'}</h3><p className="wa-ui-note wa-ui-identity">{selected ? selected.chat_id : 'Choose a recipient and message type.'}</p></div></header>
        <div className="wa-ui-cloud-reader-body">
          {selected && <CloudMessages key={`messages:${selected.chat_id}`} host={host} chat={selected} />}
          <CloudComposer key={`composer:${selected?.chat_id ?? 'new'}`} host={host} chat={selected} ready={ready} onDirty={onDirty} onSent={chats.refresh} />
        </div>
      </> : <EmptyState icon={Cloud} title="Cloud API conversations" description="Select a conversation or write a new message from your business number." action={{ label: 'New message', onClick: () => openChat(null) }} />}</PageMain>}
    </PageBody>
  </div>
}

function CloudMessages({ host, chat }: { host: Host; chat: ChatTarget }) {
  const archive = useInboxPages({ identity: chat.chat_id,
    fetchPage: async (cursor?: string) => { const page = await host.iii.trigger<MessagePage>('whatsapp::cloud::messages::list', { chat_id: chat.chat_id, cursor, limit: 50 }); return { ...page, items: page.messages } },
    merge: (previous, incoming) => mergeMessages(previous, incoming, chat.chat_id) })
  return <section className="wa-ui-cloud-messages" aria-label={`Stored Cloud messages with ${chat.title || chat.chat_id}`}>
    <div className="wa-ui-cloud-history-heading"><span className="wa-ui-note">Recent captured messages</span><IconButton label="Refresh Cloud messages" onClick={archive.refresh} disabled={archive.loading}><RefreshCw size={16} /></IconButton></div>
    {archive.error ? <StatusPanel variant="alert" headline="Could not load Cloud messages" detail={archive.error} action={<Button variant="ghost" onClick={archive.refresh}>Try again</Button>} /> : archive.loading ? <Skeleton className="wa-ui-loading" /> : !archive.items.length ? <EmptyState compact icon={MessageCircle} title="No stored messages" description="This worker has not captured messages for this recipient yet." /> : <>
      {archive.nextCursor && <div className="wa-ui-load-older"><Button variant="ghost" onClick={archive.more} disabled={archive.loadingMore}>{archive.loadingMore ? 'Loading…' : 'Load older stored messages'}</Button></div>}
      <div className="wa-ui-message-thread" role="log" aria-label="Cloud message history">{archive.items.map((message) => <article key={messageKey(message)} className="wa-ui-message-row" data-outgoing={message.from_me || undefined}><div className="wa-ui-message-bubble"><p className="wa-ui-message-text">{messageLabel(message)}</p><footer className="wa-ui-message-meta"><time dateTime={new Date(message.timestamp_ms).toISOString()}>{new Date(message.timestamp_ms).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</time>{message.from_me && <span className="wa-ui-message-delivery"><Check size={16} aria-hidden="true" />{messageStatus(message.status)}</span>}</footer></div></article>)}</div>
    </>}
  </section>
}

function CloudComposer({ host, chat, ready, onDirty, onSent }: { host: Host; chat: ChatTarget | null; ready: boolean; onDirty(value: boolean): void; onSent(): void }) {
  const [recipient, setRecipient] = useState('')
  const [mode, setMode] = useState<'text' | 'template'>('text')
  const [text, setText] = useState('')
  const [components, setComponents] = useState('')
  const [templates, setTemplates] = useState<CloudTemplate[]>([])
  const [templateId, setTemplateId] = useState('')
  const [templateCursor, setTemplateCursor] = useState<string | null>(null)
  const [templatesLoaded, setTemplatesLoaded] = useState(false)
  const [loadingTemplates, setLoadingTemplates] = useState(false)
  const [templateError, setTemplateError] = useState<string | null>(null)
  const [sending, setSending] = useState(false)
  const [sendError, setSendError] = useState<string | null>(null)
  const [sent, setSent] = useState<CloudSent | null>(null)
  const busy = useRef(false)
  const templateBusy = useRef(false)
  const sendScope = useRef(requestScope())
  const templateScope = useRef(requestScope())
  const instance = useId()
  const selected = templates.find((item) => item.id === templateId)
  const dirty = sending || !!text || !!components || !!templateId || !!recipient
  useEffect(() => { onDirty(dirty); return () => onDirty(false) }, [dirty, onDirty])
  useEffect(() => () => { sendScope.current.invalidate(); templateScope.current.invalidate() }, [])
  const edit = () => { setSendError(null); setSent(null) }
  const loadTemplates = async (more = false) => {
    if (!ready || templateBusy.current) return
    templateBusy.current = true
    const current = templateScope.current.begin()
    setLoadingTemplates(true); setTemplateError(null)
    try {
      const page = await host.iii.trigger<TemplatePage>('whatsapp::cloud::templates::list', { limit: 50, after: more ? templateCursor ?? undefined : undefined })
      if (!current()) return
      setTemplates((previous) => [...new Map([...(more ? previous : []), ...page.templates].map((item) => [item.id, item])).values()])
      setTemplateCursor(page.next_cursor); setTemplatesLoaded(true)
    } catch (cause) { if (current()) setTemplateError(errorMessage(cause)) }
    finally { if (current()) { templateBusy.current = false; setLoadingTemplates(false) } }
  }
  const send = async () => {
    if (!ready || busy.current) return
    setSendError(null); setSent(null)
    let chatId: string
    let parameters: ReturnType<typeof templateComponents>
    try {
      chatId = cloudRecipient(chat?.chat_id ?? recipient)
      if (mode === 'text') { const error = cloudTextError(text); if (error) throw new Error(error) }
      else {
        if (!selected || selected.status !== 'APPROVED') throw new Error('Choose an approved template before sending.')
        parameters = templateComponents(components)
      }
    } catch (cause) { setSendError(errorMessage(cause)); return }
    busy.current = true
    const current = sendScope.current.begin()
    setSending(true)
    try {
      const result = mode === 'text'
        ? await host.iii.trigger<CloudSent>('whatsapp::cloud::messages::send_text', { chat_id: chatId, text })
        : await host.iii.trigger<CloudSent>('whatsapp::cloud::messages::send_template', { chat_id: chatId, name: selected!.name, language: selected!.language, components: parameters })
      if (!current()) return
      setText(''); setComponents(''); setTemplateId(''); setRecipient(''); setSent(result); onSent()
    } catch (cause) { if (current()) setSendError(errorMessage(cause)) }
    finally { if (current()) { busy.current = false; setSending(false) } }
  }
  return <section className="wa-ui-cloud-composer" aria-label="Send a Cloud message">
    <div className="wa-ui-composer-heading"><h3 className="wa-ui-title">Send a message</h3><Select aria-label="Cloud message type" value={mode} options={[{ value: 'text', label: 'Text message' }, { value: 'template', label: 'Approved template' }]} onChange={(value) => { setMode(value as 'text' | 'template'); edit() }} disabled={sending} /></div>
    {!chat && <SettingsList><SettingsField field="cloud-recipient" label="Recipient" description="International phone number or business-scoped user ID." controlSize="full" renderControl={(control) => <Input {...control} value={recipient} onChange={(value) => { setRecipient(value); edit() }} placeholder="+5511999999999" autoComplete="off" disabled={sending} />} /></SettingsList>}
    {mode === 'text' ? <>
      <p className="wa-ui-note">Free-form replies require an open customer service window. Use an approved template to start a conversation outside that window.</p>
      <div className="wa-ui-composer-surface"><label className="wa-ui-composer-label" htmlFor={`${instance}-text`}>Message</label><CodeEditor id={`${instance}-text`} language="plaintext" value={text} onChange={(value) => { setText(value); edit() }} disabled={sending} className="wa-ui-compose-editor" aria-label="Cloud message text" placeholder="Write a message…" /></div>
      <span className="wa-ui-note">{Array.from(text).length.toLocaleString('en-US')} / 4,096 characters</span>
    </> : <div className="wa-ui-stack">
      <div className="wa-ui-cloud-template-actions"><Button variant="ghost" onClick={() => void loadTemplates()} disabled={!ready || sending || loadingTemplates}>{loadingTemplates ? 'Loading templates…' : templatesLoaded ? 'Refresh templates' : 'Load templates'}</Button>{templateCursor && <Button variant="ghost" onClick={() => void loadTemplates(true)} disabled={!ready || sending || loadingTemplates}>Load more templates</Button>}</div>
      {templateError && <StatusPanel variant="alert" headline="Could not load templates" detail={templateError} />}
      {templatesLoaded && !templates.length ? <EmptyState compact title="No templates found" description="Create and approve a template in WhatsApp Manager, then refresh this list." /> : <Select aria-label="Cloud message template" value={templateId || undefined} onChange={(value) => { setTemplateId(value); setComponents(''); edit() }} placeholder="Choose an approved template" disabled={sending || !templates.length} options={templates.map((item) => ({ value: item.id, label: `${item.name} · ${item.language}`, description: `${item.status} · ${item.category}`, disabled: item.status !== 'APPROVED' }))} />}
      {selected && <><p className="wa-ui-note">{selected.language} · {selected.category} · {selected.parameter_format === 'NAMED' ? 'Named parameters' : 'Positional parameters'}</p>{templateText(selected) && <p className="wa-ui-cloud-template-preview">{templateText(selected)}</p>}<details className="wa-ui-cloud-setup"><summary>Template parameters (JSON)<ChevronDown size={16} aria-hidden="true" /></summary><div className="wa-ui-stack"><p className="wa-ui-note">Enter the component objects required by this template. Leave empty when no parameters are needed.</p><CodeEditor language="json" value={components} onChange={(value) => { setComponents(value); edit() }} aria-label="Cloud template parameters" className="wa-ui-cloud-json" disabled={sending} placeholder="[]" /></div></details></>}
    </div>}
    {sendError && <StatusPanel variant="alert" headline="Message not confirmed" detail={sendError} />}
    {sent && <StatusPanel className="wa-ui-cloud-result" variant="success" headline="Accepted by Meta" detail={`Recipient: ${sent.chat_id}. Acceptance does not confirm delivery; webhook updates appear in the archive.`} />}
    <div className="wa-ui-cloud-send"><p className="wa-ui-note">{!ready ? 'Enable and configure the Cloud API before sending.' : 'Sends only when you choose Send.'}</p><Button onClick={() => void send()} disabled={!ready || sending || loadingTemplates || (!chat && !recipient.trim()) || (mode === 'text' ? !text.trim() : selected?.status !== 'APPROVED')}><Send size={16} aria-hidden="true" />{sending ? 'Sending…' : mode === 'template' ? 'Send template' : 'Send'}</Button></div>
  </section>
}
