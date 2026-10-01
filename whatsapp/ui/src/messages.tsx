import { Badge, Button, CodeEditor, EmptyState, IconButton, Input, List, ListItem, LiveRegion, PageBody, PageMain, PageSidebar, SearchField, Skeleton, StatusPanel, type Host, type LiveAnnouncement } from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useDebounce } from '@iii-dev/console-ui/hooks'
import { ArrowLeft, Check, History, MessageCircle, MessageSquarePlus, Paperclip, RefreshCw, Send, Users } from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { coverageText, identityLabel, isMediaMessage, mergeChats, mergeMessages, messageKey, messageLabel, messageStatus, type ChatPage, type ChatTarget, type MessagePage, type MessageView } from './inbox-model'
import { normalizeRecipient, requestScope, validateMessage, type SentMessage } from './model'
import { useInboxPages } from './use-inbox'

const dateLabel = (timestamp: number) => new Date(timestamp).toLocaleDateString('pt-BR', { day: 'numeric', month: 'long', year: 'numeric' })
const timeLabel = (timestamp: number) => new Date(timestamp).toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' })

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
    {(!narrow || !selected) && <PageSidebar label="Conversas" side={panelSide} narrow={narrow} defaultWidth={300} minWidth={220} maxWidth={360} header="Conversas">
      <div className="wa-ui-list-tools">
        <SearchField aria-label="Buscar conversas" placeholder="Buscar conversas" value={query} onChange={setQuery} />
        <div className="wa-ui-list-actions"><span className="wa-ui-note">{list.items.length} conversas disponíveis</span><IconButton label="Nova mensagem" onClick={() => setNewChat((value) => !value)}><MessageSquarePlus size={16} /></IconButton><IconButton label="Atualizar conversas" onClick={list.refresh} disabled={list.loading}><RefreshCw size={16} /></IconButton></div>
        {newChat && <form onSubmit={openRecipient} className="wa-ui-stack">
          <Input autoFocus aria-label="Destinatário da nova conversa" value={recipient} onChange={(value) => { setRecipient(value); setRecipientError(null) }} placeholder="+5511999999999 ou JID" autoComplete="off" />
          {recipientError && <StatusPanel variant="alert" headline={recipientError} />}
          <Button type="submit" disabled={!recipient.trim()}>Abrir conversa</Button>
        </form>}
      </div>
      {list.error && <div className="wa-ui-list-tools"><StatusPanel variant="alert" headline="Não foi possível carregar as conversas" detail={list.error} action={<Button variant="ghost" onClick={list.refresh}>Tentar novamente</Button>} /></div>}
      <div className="wa-ui-list">
        {list.loading ? <><Skeleton className="wa-ui-loading" /><Skeleton className="wa-ui-loading" /></> : list.items.length === 0 ? <EmptyState compact icon={MessageCircle} title={query ? 'Nenhuma conversa encontrada' : 'Ainda não há mensagens armazenadas'} description={query ? 'Tente outro nome ou identificador.' : 'As mensagens recebidas e o histórico disponível do WhatsApp aparecerão aqui.'} /> : <List aria-label="Conversas disponíveis">{list.items.map((chat) => <ListItem key={chat.chat_id} selected={selected?.chat_id === chat.chat_id}
          leading={chat.is_group ? <Users size={16} aria-hidden="true" /> : <MessageCircle size={16} aria-hidden="true" />}
          label={<span className="wa-ui-chat-title">{chat.title || identityLabel(chat.chat_id)}</span>}
          description={<span className="wa-ui-preview">{chat.last_message ? `${chat.last_message.from_me ? 'Você: ' : ''}${messageLabel(chat.last_message)}` : 'Sem prévia de mensagem'}</span>}
          trailing={<span className="wa-ui-chat-meta">{chat.last_message && <time dateTime={new Date(chat.last_message.timestamp_ms).toISOString()}>{formatRelative(chat.last_message.timestamp_ms)}</time>}<span>{chat.message_count.toLocaleString('pt-BR')}</span></span>}
          onClick={() => onSelect(chat)} />)}</List>}
        {list.nextCursor && <div className="wa-ui-list-tools"><Button variant="ghost" onClick={list.more} disabled={list.loadingMore}>{list.loadingMore ? 'Carregando…' : 'Carregar mais conversas'}</Button></div>}
      </div>
      {list.coverage && <p className="wa-ui-coverage wa-ui-note">{coverageText(list.coverage)}</p>}
    </PageSidebar>}
    {(!narrow || selected) && <PageMain className="wa-ui-reader">
      {selected ? <MessageReader key={`${sessionId}:${selected.chat_id}`} host={host} sessionId={sessionId} chat={selected} connected={connected} onBack={() => onSelect(null)} onDirty={onDirty} onGoConnection={onGoConnection} refreshChats={list.refresh} /> : <EmptyState icon={MessageCircle} title="Escolha uma conversa" description="Leia mensagens capturadas nesta sessão e o histórico recebido do WhatsApp. Para encontrar um grupo da conta, abra a aba Grupos." />}
    </PageMain>}
  </PageBody>
}

function MessageRow({ message, group }: { message: MessageView; group: boolean }) {
  const status = messageStatus(message.status)
  return <article className="wa-ui-message-row" data-outgoing={message.from_me || undefined} aria-label={`${message.from_me ? 'Você' : message.sender_name || identityLabel(message.sender_id)}, ${timeLabel(message.timestamp_ms)}`}>
    <div className="wa-ui-message-bubble">
      {(group || message.from_me) && <span className="wa-ui-message-sender">{message.from_me ? 'Você' : message.sender_name || identityLabel(message.sender_id)}</span>}
      <p className="wa-ui-message-text">{messageLabel(message)}</p>
      {isMediaMessage(message) && <span className="wa-ui-media-note"><Paperclip size={16} aria-hidden="true" />Mídia não baixada</span>}
      <footer className="wa-ui-message-meta"><time dateTime={new Date(message.timestamp_ms).toISOString()} title={new Date(message.timestamp_ms).toLocaleString('pt-BR')}>{timeLabel(message.timestamp_ms)}</time>{message.source === 'history' && <span>Histórico</span>}{message.from_me && status && <span className="wa-ui-message-delivery"><Check size={16} aria-hidden="true" />{status}</span>}</footer>
    </div>
  </article>
}

function MessageReader({ host, sessionId, chat, connected, onBack, onDirty, onGoConnection, refreshChats }: {
  host: Host; sessionId: string; chat: ChatTarget; connected: boolean; onBack(): void
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
  const [draft, setDraft] = useState('')
  const [sending, setSending] = useState(false)
  const [requesting, setRequesting] = useState(false)
  const [sendError, setSendError] = useState<string | null>(null)
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
  const instance = useId()

  useEffect(() => () => { scope.current.invalidate(); historyScope.current.invalidate() }, [])
  useEffect(() => { onDirty(!!draft.trim() || sending); return () => onDirty(false) }, [draft, sending, onDirty])
  useLayoutEffect(() => {
    const node = scroll.current
    if (!node) return
    if (olderAnchor.current) {
      node.scrollTop = olderAnchor.current.top + node.scrollHeight - olderAnchor.current.height
      olderAnchor.current = null
    } else if (followLatest.current) node.scrollTop = node.scrollHeight
  }, [archive.items])

  const send = async () => {
    if (!connected || sendBusy.current || historyBusy.current) return
    const invalid = validateMessage(draft)
    if (invalid) { setSendError(invalid.replace('mensagem de teste', 'mensagem')); return }
    sendBusy.current = true
    const current = scope.current.begin()
    setSending(true)
    setSendError(null)
    setSent(null)
    try {
      const result = await host.iii.trigger<SentMessage>('whatsapp::messages::send_text', { session_id: sessionId, chat_id: chat.chat_id, text: draft }, { timeoutMs: 610000 })
      if (!current()) return
      setDraft('')
      setSent(result)
      followLatest.current = true
      archive.refresh()
      refreshChats()
      setAnnouncement({ seq: Date.now(), text: 'Mensagem enviada.', urgency: 'polite' })
    } catch (cause) {
      if (current()) setSendError(`${errorMessage(cause)} O envio não será repetido automaticamente. Confira as mensagens antes de tentar novamente.`)
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
      setHistoryNotice(`Solicitação de até ${result.requested_count} mensagens enviada ao celular. O histórico disponível aparecerá quando o WhatsApp responder.`)
      archive.refresh()
    } catch (cause) { if (current()) setHistoryError(errorMessage(cause)) }
    finally { if (current()) { historyBusy.current = false; setRequesting(false) } }
  }
  const older = () => {
    if (scroll.current) olderAnchor.current = { height: scroll.current.scrollHeight, top: scroll.current.scrollTop }
    archive.more()
  }

  return <>
    <header className="wa-ui-detail-header">
      <IconButton label="Voltar às conversas" onClick={onBack}><ArrowLeft size={16} /></IconButton>
      <div className="wa-ui-heading-copy"><h2 className="wa-ui-title">{chat.title || identityLabel(chat.chat_id)}</h2><span className="wa-ui-note wa-ui-identity">{chat.chat_id}</span></div>
      {chat.is_group && <Badge>Grupo</Badge>}
      <IconButton label="Atualizar mensagens" onClick={archive.refresh} disabled={archive.loading}><RefreshCw size={16} /></IconButton>
      <Button variant="ghost" onClick={() => void requestHistory()} disabled={!connected || requesting || sending || archive.items.length === 0} title={archive.items.length === 0 ? 'É necessário ter uma mensagem armazenada para buscar mensagens anteriores.' : 'Solicitar mensagens anteriores ao celular'}><History size={16} aria-hidden="true" />{requesting ? 'Solicitando…' : 'Buscar histórico'}</Button>
    </header>
    {archive.error && <div className="wa-ui-reader-notice"><StatusPanel variant="alert" headline="Não foi possível atualizar as mensagens" detail={archive.error} action={<Button variant="ghost" onClick={archive.refresh}>Tentar novamente</Button>} /></div>}
    {historyError && <div className="wa-ui-reader-notice"><StatusPanel variant="alert" headline="Não foi possível buscar o histórico" detail={historyError} action={<Button variant="ghost" onClick={() => setHistoryError(null)}>Dispensar</Button>} /></div>}
    {historyNotice && <div className="wa-ui-reader-notice"><StatusPanel variant="info" headline="Histórico solicitado" detail={historyNotice} action={<Button variant="ghost" onClick={() => setHistoryNotice(null)}>Dispensar</Button>} /></div>}
    <div ref={scroll} className="wa-ui-messages" onScroll={() => { const node = scroll.current; if (node) followLatest.current = node.scrollHeight - node.scrollTop - node.clientHeight < 96 }}>
      {archive.loading ? <><Skeleton className="wa-ui-loading" /><Skeleton className="wa-ui-loading" /></> : archive.items.length === 0 ? <EmptyState compact icon={MessageCircle} title="Ainda não há mensagens desta conversa" description="Esta sessão mostra as mensagens capturadas e o histórico recebido do WhatsApp. Um grupo pode estar na conta mesmo sem mensagens disponíveis aqui." /> : <>
        {archive.nextCursor && <div className="wa-ui-load-older"><Button variant="ghost" onClick={older} disabled={archive.loadingMore}>{archive.loadingMore ? 'Carregando…' : 'Carregar mensagens anteriores'}</Button></div>}
        <div role="log" aria-label={`Mensagens de ${chat.title}`} aria-live="off">{archive.items.map((message, index) => <div key={messageKey(message)} className="wa-ui-message-entry">
          {(index === 0 || dateLabel(archive.items[index - 1].timestamp_ms) !== dateLabel(message.timestamp_ms)) && <div className="wa-ui-message-date">{dateLabel(message.timestamp_ms)}</div>}
          <MessageRow message={message} group={chat.is_group} />
        </div>)}</div>
      </>}
    </div>
    {archive.coverage && <p className="wa-ui-coverage wa-ui-note">{archive.items.length.toLocaleString('pt-BR')} mensagens nesta visualização. {coverageText(archive.coverage)}</p>}
    <section className="wa-ui-composer" aria-label="Escrever mensagem">
      {!connected && <StatusPanel variant="warn" headline="Sessão desconectada" detail="Você pode ler o arquivo disponível. Conecte a sessão para enviar mensagens ou buscar histórico." action={<Button variant="ghost" onClick={onGoConnection}>Abrir conexão</Button>} />}
      {sendError && <StatusPanel variant="alert" headline="A mensagem não foi confirmada" detail={sendError} action={<Button variant="ghost" onClick={() => setSendError(null)}>Dispensar</Button>} />}
      <label htmlFor={`${instance}-composer`} className="wa-ui-composer-label">Mensagem para {chat.title || identityLabel(chat.chat_id)}</label>
      <CodeEditor id={`${instance}-composer`} language="plaintext" value={draft} onChange={(value) => { setDraft(value); setSendError(null); setSent(null) }} disabled={sending} aria-label="Mensagem" className="wa-ui-compose-editor" placeholder="Escreva uma mensagem…" />
      <div className="wa-ui-composer-actions"><span className="wa-ui-note">{sent ? `Submetida: ${sent.message_id}. A entrega será atualizada na conversa.` : 'O envio acontece apenas ao escolher Enviar.'}</span><Button onClick={() => void send()} disabled={!connected || sending || requesting || !draft.trim()}><Send size={16} aria-hidden="true" />{sending ? 'Enviando…' : 'Enviar'}</Button></div>
    </section>
    <LiveRegion announcement={announcement} />
  </>
}
