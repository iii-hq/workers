import { Button, EmptyState, IconButton, Input, List, ListItem, PageBody, PageHeader, PageMain, PageShell, PageSidebar, SettingsField, Skeleton, StatusBar, StatusPanel, Toolbar, useConfirm, type Host, type PageRenderProps } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { ArrowLeft, MessageCircle, Plus, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { RuntimeStatus } from './configuration'
import { requestScope, type Session } from './model'
import { SessionBadge, SessionDetail } from './session'

export function WhatsAppPage({ host, configurationId, paneId, tabId, panelSide, onRequestClose, setDirty, commands }: PageRenderProps & { host: Host; configurationId: string }) {
  const stateKey = `whatsapp:${configurationId}:${paneId || tabId || 'standalone'}`
  const [selected, setSelected] = usePaneState<string | null>(`${stateKey}:session`, null)
  const [newId, setNewId] = usePaneState(`${stateKey}:new-session`, '')
  const [createError, setCreateError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [dirty, setDetailDirty] = useState(false)
  const busy = useRef(false)
  const scope = useRef(requestScope())
  const { ref, narrow } = useContainerNarrow()
  const { confirm, dialog } = useConfirm()
  const newIdRef = useRef<HTMLInputElement>(null)
  const backRef = useRef<HTMLButtonElement>(null)
  const focusNewRequested = useRef(false)
  const instance = useId()
  const list = useWorkerLive<Session[]>({
    iii: host.iii,
    triggers: [{ type: 'whatsapp::events', config: { kinds: ['session.created', 'session.connected', 'session.disconnected', 'session.stopped', 'session.starting', 'session.qr', 'session.logged_out'] } }],
    handlerId: `iii::whatsapp-ui::sessions::${instance}`, pollMs: 5000,
    fetch: () => host.iii.trigger<Session[]>('whatsapp::sessions::list'),
  })
  // Best-effort events need periodic reconciliation even when the live binding is available.
  useEffect(() => {
    const timer = window.setInterval(() => { if (!document.hidden) list.refresh() }, 10000)
    return () => window.clearInterval(timer)
  }, [list.refresh])
  useEffect(() => () => scope.current.invalidate(), [])
  const onDirty = useCallback((next: boolean) => setDetailDirty(next), [])
  useEffect(() => { setDirty?.(dirty ? 'Mensagem de teste do WhatsApp' : false); return () => setDirty?.(false) }, [setDirty, dirty])
  useEffect(() => { if (selected && narrow) backRef.current?.focus() }, [selected, narrow])
  useEffect(() => {
    if (focusNewRequested.current && (!narrow || !selected)) {
      newIdRef.current?.focus()
      focusNewRequested.current = false
    }
  }, [selected, narrow])

  const mayLeave = async () => !dirty || confirm({ title: 'Descartar o rascunho da mensagem?', description: 'A mensagem de teste ainda não foi enviada. Ao trocar de sessão, o destinatário e o texto serão descartados.', confirmLabel: 'Descartar rascunho', cancelLabel: 'Continuar editando' })
  const select = async (id: string | null) => {
    if (id === selected || busy.current) return
    busy.current = true
    const current = scope.current.begin()
    try {
      if (await mayLeave() && current()) { setDetailDirty(false); setSelected(id) }
    } finally { busy.current = false }
  }
  const create = async () => {
    if (busy.current) return
    const id = newId.trim()
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(id)) { setCreateError('Use de 1 a 64 letras, números, hífens ou sublinhados.'); newIdRef.current?.focus(); return }
    busy.current = true
    const current = scope.current.begin()
    try {
      if (!await mayLeave() || !current()) return
      setCreating(true)
      setCreateError(null)
      await host.iii.trigger<Session>('whatsapp::sessions::create', { session_id: id })
      if (!current()) return
      setDetailDirty(false)
      setSelected(id)
      setNewId('')
      list.refresh()
    } catch (cause) { if (current()) setCreateError(errorMessage(cause)) }
    finally { if (current()) { busy.current = false; setCreating(false) } }
  }
  const focusNew = async () => {
    if (narrow && selected) { focusNewRequested.current = true; await select(null) }
    else newIdRef.current?.focus()
  }
  useEffect(() => commands?.register([
    { id: 'refresh', title: 'Atualizar sessões', shortcut: 'R', run: list.refresh },
    { id: 'new', title: 'Criar sessão', shortcut: 'N', run: () => void focusNew() },
    { id: 'back', title: 'Voltar às sessões', enabled: () => selected !== null, run: () => void select(null) },
  ]), [commands, list.refresh, narrow, selected, dirty])
  const sessions = list.data ?? []

  return <PageShell ref={ref} className="wa-ui-shell">
    <PageHeader icon={<MessageCircle size={16} />} title="WhatsApp" description="Sessões e mensagens" onClose={onRequestClose}
      actions={<IconButton label="Atualizar sessões" onClick={list.refresh} disabled={list.loading}><RefreshCw size={16} /></IconButton>} />
    <PageBody side={panelSide}>
      {(!narrow || !selected) && <PageSidebar label="Sessões do WhatsApp" side={panelSide} narrow={narrow} header="Sessões">
        <form className="wa-ui-create" onSubmit={(event) => { event.preventDefault(); void create() }}>
          <SettingsField field="session_id" label="Identificador da sessão" error={createError ?? undefined} description="Um nome local para esta conta." layout="stacked" controlSize="full" renderControl={(control) => <Input {...control} ref={newIdRef} data-autofocus value={newId} onChange={(next) => { setNewId(next); setCreateError(null) }} placeholder="atendimento" maxLength={64} disabled={creating} autoComplete="off" spellCheck={false} />} />
          <Button type="submit" disabled={creating || !newId.trim()}><Plus size={16} aria-hidden="true" />{creating ? 'Criando…' : 'Criar sessão'}</Button>
          <p className="wa-ui-note">Criar salva a sessão sem conectar a conta.</p>
        </form>
        {list.error && <StatusPanel variant="alert" headline="Não foi possível listar as sessões" detail={list.error} action={<Button variant="ghost" onClick={list.refresh}>Tentar novamente</Button>} />}
        {list.loading && !list.data ? <Skeleton className="wa-ui-loading" /> : sessions.length === 0 ? <EmptyState compact title="Nenhuma sessão criada" description="Escolha um identificador acima para começar. Depois, inicie e pareie sua conta." /> : <List aria-label="Sessões">{sessions.map((session) => <ListItem key={session.session_id} label={<span className="wa-ui-mono">{session.session_id}</span>} selected={selected === session.session_id} trailing={<SessionBadge status={session.status} />} onClick={() => void select(session.session_id)} />)}</List>}
      </PageSidebar>}
      {(!narrow || selected) && <PageMain className="wa-ui-main">
        {selected && <Toolbar aria-label="Navegação da sessão"><Button variant="ghost" ref={backRef} data-autofocus={narrow || undefined} onClick={() => void select(null)}><ArrowLeft size={16} aria-hidden="true" />Sessões</Button></Toolbar>}
        <div className="wa-ui-scroll">
          <div className="wa-ui-runtime"><RuntimeStatus host={host} quietWhenApplied /></div>
          {selected ? <SessionDetail key={selected} host={host} sessionId={selected} refreshList={list.refresh} onDirty={onDirty} commands={commands} /> : <EmptyState icon={MessageCircle} title="Conecte uma conta ao WhatsApp" description="Crie ou selecione uma sessão. Inicie a conexão, pareie pelo QR ou telefone e envie uma mensagem de teste. Ajuste os limites na configuração desta página." action={{ label: 'Criar sessão', onClick: () => void focusNew() }} />}
        </div>
        <StatusBar aria-label="Estado do WhatsApp">{list.error ? 'Atualização indisponível' : list.live ? 'Atualizações de sessão ativas' : 'Consultando sessões periodicamente'}<span className="wa-ui-mono">{sessions.length} sessões</span></StatusBar>
      </PageMain>}
    </PageBody>
    {dialog}
  </PageShell>
}
