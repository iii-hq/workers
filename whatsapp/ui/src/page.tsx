import { Button, EmptyState, IconButton, Input, PageHeader, PageShell, Select, SettingsField, Skeleton, StatusBar, StatusPanel, Tabs, TabsContent, TabsList, TabsTrigger, Toolbar, useConfirm, type Host, type PageRenderProps } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Cable, MessageCircle, Plus, RefreshCw, Users } from 'lucide-react'
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { RuntimeStatus } from './configuration'
import { GroupBrowser } from './groups'
import type { ChatTarget } from './inbox-model'
import { MessageBrowser } from './messages'
import { requestScope, STATUS_LABEL, type Session } from './model'
import { SessionBadge, SessionDetail } from './session'

type View = 'messages' | 'groups' | 'connection'

export function WhatsAppPage({ host, configurationId, paneId, tabId, panelSide, onRequestClose, setDirty, commands }: PageRenderProps & { host: Host; configurationId: string }) {
  const stateKey = `whatsapp:${configurationId}:${paneId || tabId || 'standalone'}`
  const [selected, setSelected] = usePaneState<string | null>(`${stateKey}:session`, null)
  const [newId, setNewId] = useState('')
  const [showCreate, setShowCreate] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [dirty, setDetailDirty] = useState(false)
  const [view, setView] = useState<View>('messages')
  const [chat, setChat] = useState<ChatTarget | null>(null)
  const busy = useRef(false)
  const scope = useRef(requestScope())
  const { ref, narrow } = useContainerNarrow({ below: 760 })
  const { confirm, dialog } = useConfirm()
  const newIdRef = useRef<HTMLInputElement>(null)
  const instance = useId()
  const list = useWorkerLive<Session[]>({
    iii: host.iii,
    triggers: [{ type: 'whatsapp::events', config: { kinds: ['session.created', 'session.connected', 'session.disconnected', 'session.stopped', 'session.starting', 'session.qr', 'session.logged_out'] } }],
    handlerId: `iii::whatsapp-ui::sessions::${instance}`, pollMs: 5000,
    fetch: () => host.iii.trigger<Session[]>('whatsapp::sessions::list'),
  })
  const sessions = list.data ?? []
  const session = sessions.find((item) => item.session_id === selected)
  const connected = session?.status === 'connected' && !list.error
  const accountIdentity = `${selected}:${session?.created_at_ms ?? ''}:${session?.status === 'logged_out' ? 'logged-out' : 'active'}`

  useEffect(() => {
    const timer = window.setInterval(() => { if (!document.hidden) list.refresh() }, 10000)
    return () => window.clearInterval(timer)
  }, [list.refresh])
  useEffect(() => () => scope.current.invalidate(), [])
  useEffect(() => {
    if (selected === null && list.data?.length) setSelected((list.data.find((item) => item.status === 'connected') ?? list.data[0]).session_id)
  }, [list.data, selected, setSelected])
  const onDirty = useCallback((next: boolean) => setDetailDirty(next), [])
  useEffect(() => { setDirty?.(dirty ? 'Mensagem do WhatsApp' : false); return () => setDirty?.(false) }, [setDirty, dirty])
  useEffect(() => { if (showCreate) newIdRef.current?.focus() }, [showCreate])
  useEffect(() => {
    if (session?.status === 'logged_out') { setChat(null); setDetailDirty(false) }
  }, [session?.status])

  const navigate = async (action: () => void): Promise<boolean> => {
    if (busy.current) return false
    busy.current = true
    const current = scope.current.begin()
    try {
      const accepted = !dirty || await confirm({ title: 'Descartar o rascunho da mensagem?', description: 'Ao sair desta conversa, o texto ainda não enviado será descartado. Se um envio já foi solicitado, ele poderá ser concluído.', confirmLabel: 'Descartar rascunho', cancelLabel: 'Continuar editando' })
      if (accepted && current()) { setDetailDirty(false); action(); return true }
      return false
    } finally { if (current()) busy.current = false }
  }
  const selectSession = (id: string) => {
    if (id === selected) return
    void navigate(() => { setSelected(id); setChat(null) })
  }
  const selectChat = (next: ChatTarget | null): Promise<boolean> => {
    if (next?.chat_id === chat?.chat_id) return Promise.resolve(true)
    return navigate(() => setChat(next))
  }
  const selectView = (next: View) => {
    if (next === view) return
    void navigate(() => setView(next))
  }
  const openGroup = (target: ChatTarget) => { void navigate(() => { setChat(target); setView('messages') }) }
  const create = async () => {
    if (busy.current) return
    const id = newId.trim()
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(id)) { setCreateError('Use de 1 a 64 letras, números, hífens ou sublinhados.'); newIdRef.current?.focus(); return }
    if (!await navigate(() => setChat(null))) return
    busy.current = true
    const current = scope.current.begin()
    setCreating(true)
    setCreateError(null)
    try {
      await host.iii.trigger<Session>('whatsapp::sessions::create', { session_id: id })
      if (!current()) return
      setSelected(id)
      setChat(null)
      setView('connection')
      setNewId('')
      setShowCreate(false)
      list.refresh()
    } catch (cause) { if (current()) setCreateError(errorMessage(cause)) }
    finally { if (current()) { busy.current = false; setCreating(false) } }
  }
  useEffect(() => commands?.register([
    { id: 'refresh', title: 'Atualizar sessões', shortcut: 'R', run: list.refresh },
    { id: 'new', title: 'Criar sessão', shortcut: 'N', run: () => setShowCreate(true) },
    { id: 'messages', title: 'Abrir mensagens', run: () => selectView('messages') },
    { id: 'groups', title: 'Abrir grupos', run: () => selectView('groups') },
    { id: 'connection', title: 'Abrir conexão', run: () => selectView('connection') },
  ]), [commands, list.refresh, dirty, view])

  return <PageShell ref={ref} className="wa-ui-shell">
    <PageHeader icon={<MessageCircle size={16} />} title="WhatsApp" description="Mensagens e grupos" onClose={onRequestClose}
      actions={<IconButton label="Atualizar sessões" onClick={list.refresh} disabled={list.loading}><RefreshCw size={16} /></IconButton>} />
    <Toolbar className="wa-ui-account-bar" aria-label="Conta e sessão">
      <div className="wa-ui-session-select"><Select aria-label="Sessão do WhatsApp" value={selected ?? undefined} options={sessions.map((item) => ({ value: item.session_id, label: item.session_id, description: STATUS_LABEL[item.status] }))} onChange={selectSession} placeholder="Escolha uma sessão" disabled={creating} /></div>
      {session && <SessionBadge status={session.status} />}
      <Button variant="ghost" onClick={() => setShowCreate((value) => !value)} disabled={creating}><Plus size={16} aria-hidden="true" />Nova sessão</Button>
    </Toolbar>
    {(showCreate || (!list.loading && sessions.length === 0)) && <form className="wa-ui-new-session" onSubmit={(event) => { event.preventDefault(); void create() }}>
      <SettingsField field="session_id" label="Nome da sessão" error={createError ?? undefined} description="Criar salva a sessão. Você inicia e pareia a conta na aba Conexão." controlSize="full" renderControl={(control) => <Input {...control} ref={newIdRef} value={newId} onChange={(next) => { setNewId(next); setCreateError(null) }} placeholder="atendimento" maxLength={64} disabled={creating} autoComplete="off" spellCheck={false} />} />
      <Button type="submit" disabled={creating || !newId.trim()}>{creating ? 'Criando…' : 'Criar sessão'}</Button>
    </form>}
    {list.error && <div className="wa-ui-reader-notice"><StatusPanel variant="alert" headline="Não foi possível atualizar a sessão" detail={list.error} action={<Button variant="ghost" onClick={list.refresh}>Tentar novamente</Button>} /></div>}
    <div className="wa-ui-runtime"><RuntimeStatus host={host} quietWhenApplied /></div>
    {selected ? <Tabs value={view} onValueChange={(value) => selectView(value as View)} className="wa-ui-tabs">
      <TabsList className="wa-ui-tab-list" aria-label="WhatsApp"><TabsTrigger value="messages" icon={<MessageCircle size={16} />}>Mensagens</TabsTrigger><TabsTrigger value="groups" icon={<Users size={16} />}>Grupos</TabsTrigger><TabsTrigger value="connection" icon={<Cable size={16} />}>Conexão</TabsTrigger></TabsList>
      <TabsContent value="messages" className="wa-ui-tab-content"><MessageBrowser key={accountIdentity} host={host} sessionId={selected} connected={connected} narrow={narrow} panelSide={panelSide} selected={chat} onSelect={selectChat} onDirty={onDirty} onGoConnection={() => selectView('connection')} /></TabsContent>
      <TabsContent value="groups" className="wa-ui-tab-content"><GroupBrowser key={accountIdentity} host={host} sessionId={selected} connected={connected} narrow={narrow} panelSide={panelSide} onOpenMessages={openGroup} /></TabsContent>
      <TabsContent value="connection" className="wa-ui-tab-content wa-ui-connection-scroll"><SessionDetail key={`${selected}:${session?.created_at_ms ?? ''}`} host={host} sessionId={selected} refreshList={list.refresh} commands={commands} /></TabsContent>
    </Tabs> : list.loading ? <Skeleton className="wa-ui-loading" /> : <EmptyState icon={MessageCircle} title="Conecte uma conta ao WhatsApp" description="Crie uma sessão e pareie a conta na aba Conexão. Depois, acompanhe suas conversas e os grupos dos quais participa." />}
    <StatusBar aria-label="Estado do WhatsApp">{session ? `${session.session_id} · ${STATUS_LABEL[session.status]}` : 'Nenhuma sessão selecionada'}<span>{view === 'messages' ? 'Arquivo disponível nesta sessão' : view === 'groups' ? 'Grupos dos quais a conta participa' : 'A conexão começa apenas ao escolher Iniciar'}</span></StatusBar>
    {dialog}
  </PageShell>
}
