import { Button, EmptyState, IconButton, Input, PageHeader, PageShell, Select, SettingsField, SettingsList, Skeleton, StatusBar, StatusPanel, Tabs, TabsContent, TabsList, TabsTrigger, useConfirm, type Host, type PageRenderProps } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Cable, Cloud, MessageCircle, Plus, RefreshCw, Users } from 'lucide-react'
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { RuntimeStatus } from './configuration'
import { CloudPanel } from './cloud'
import { GroupBrowser } from './groups'
import type { ChatTarget } from './inbox-model'
import { MessageBrowser } from './messages'
import { requestScope, STATUS_LABEL, type Session } from './model'
import { SessionBadge, SessionDetail } from './session'

type View = 'messages' | 'groups' | 'connection' | 'cloud'

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
  const shell = useRef<HTMLDivElement>(null)
  const shellRef = useCallback((node: HTMLDivElement | null) => { shell.current = node; ref(node) }, [ref])
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
    const strip = shell.current?.querySelector<HTMLElement>('.wa-ui-tab-list')
    if (!strip) return
    const revealActive = () => {
      const active = strip.querySelector<HTMLElement>('[role="tab"][data-state="active"]')
      if (!active) return
      const listBounds = strip.getBoundingClientRect(), tabBounds = active.getBoundingClientRect()
      if (tabBounds.left < listBounds.left) strip.scrollLeft += tabBounds.left - listBounds.left
      else if (tabBounds.right > listBounds.right) strip.scrollLeft += tabBounds.right - listBounds.right
    }
    const observer = new ResizeObserver(revealActive)
    observer.observe(strip)
    revealActive()
    return () => observer.disconnect()
  }, [view])
  useEffect(() => {
    if (selected === null && list.data?.length) setSelected((list.data.find((item) => item.status === 'connected') ?? list.data[0]).session_id)
  }, [list.data, selected, setSelected])
  const onDirty = useCallback((next: boolean) => setDetailDirty(next), [])
  useEffect(() => { setDirty?.(dirty ? 'WhatsApp message' : false); return () => setDirty?.(false) }, [setDirty, dirty])
  useEffect(() => { if (showCreate) newIdRef.current?.focus() }, [showCreate])
  useEffect(() => {
    if (session?.status === 'logged_out') { setChat(null); if (view !== 'cloud') setDetailDirty(false) }
  }, [session?.status, view])

  const navigate = async (action: () => void): Promise<boolean> => {
    if (busy.current) return false
    busy.current = true
    const current = scope.current.begin()
    try {
      const accepted = !dirty || await confirm({ title: 'Discard this draft?', description: 'Leaving will discard your unsent message. A send already requested may still complete.', confirmLabel: 'Discard draft', cancelLabel: 'Keep editing' })
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
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(id)) { setCreateError('Use 1-64 letters, numbers, hyphens or underscores.'); newIdRef.current?.focus(); return }
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
    { id: 'refresh', title: 'Refresh sessions', shortcut: 'R', run: list.refresh },
    { id: 'new', title: 'Create session', shortcut: 'N', run: () => setShowCreate(true) },
    { id: 'messages', title: 'Open messages', run: () => selectView('messages') },
    { id: 'groups', title: 'Open groups', run: () => selectView('groups') },
    { id: 'connection', title: 'Open connection', run: () => selectView('connection') },
    { id: 'cloud', title: 'Open Cloud API', run: () => selectView('cloud') },
  ]), [commands, list.refresh, dirty, view])

  return <PageShell ref={shellRef} className="wa-ui-shell">
    <PageHeader className="wa-ui-navigation" icon={<MessageCircle size={16} />} title="WhatsApp" onClose={onRequestClose}
      actions={<><IconButton label="New session" onClick={() => setShowCreate((value) => !value)} disabled={creating}><Plus size={16} /></IconButton><IconButton className="wa-ui-session-refresh" label="Refresh sessions" onClick={list.refresh} disabled={list.loading}><RefreshCw size={16} /></IconButton></>}>
      {view === 'cloud' ? <span className="wa-ui-note">WhatsApp Business Platform</span> : <div className="wa-ui-account-bar" aria-label="Account and session">
        <div className="wa-ui-session-select"><Select aria-label={`WhatsApp session: ${selected ?? 'Choose a session'}`} value={selected ?? undefined} options={sessions.map((item) => ({ value: item.session_id, label: item.session_id, description: STATUS_LABEL[item.status] }))} onChange={selectSession} placeholder="Choose a session" disabled={creating} /></div>
        {session && <span className="wa-ui-account-status"><SessionBadge status={session.status} /></span>}
      </div>}
    </PageHeader>
    {(showCreate || (view !== 'cloud' && !list.loading && sessions.length === 0)) && <form className="wa-ui-new-session" onSubmit={(event) => { event.preventDefault(); void create() }}>
      <SettingsList><SettingsField field="session_id" label="Session name" error={createError ?? undefined} description="Create a session, then link your phone in Connection." controlSize="full" renderControl={(control) => <Input {...control} ref={newIdRef} value={newId} onChange={(next) => { setNewId(next); setCreateError(null) }} placeholder="support" maxLength={64} disabled={creating} autoComplete="off" spellCheck={false} />} /></SettingsList>
      <Button type="submit" disabled={creating || !newId.trim()}>{creating ? 'Creating…' : 'Create session'}</Button>
    </form>}
    {list.error && <div className="wa-ui-reader-notice"><StatusPanel variant="alert" headline="Could not refresh sessions" detail={list.error} action={<Button variant="ghost" onClick={list.refresh}>Try again</Button>} /></div>}
    <div className="wa-ui-runtime"><RuntimeStatus host={host} quietWhenApplied /></div>
    <Tabs value={view} onValueChange={(value) => selectView(value as View)} className="wa-ui-tabs">
      <TabsList className="wa-ui-tab-list" aria-label="WhatsApp"><TabsTrigger value="messages" icon={<MessageCircle size={16} />}>Messages</TabsTrigger><TabsTrigger value="groups" icon={<Users size={16} />}>Groups</TabsTrigger><TabsTrigger value="connection" icon={<Cable size={16} />}>Connection</TabsTrigger><TabsTrigger value="cloud" icon={<Cloud size={16} />}>Cloud API</TabsTrigger></TabsList>
      <TabsContent value="messages" className="wa-ui-tab-content">{selected ? <MessageBrowser key={accountIdentity} host={host} sessionId={selected} connected={connected} narrow={narrow} panelSide={panelSide} selected={chat} onSelect={selectChat} onDirty={onDirty} onGoConnection={() => selectView('connection')} /> : <NativeEmpty loading={list.loading} />}</TabsContent>
      <TabsContent value="groups" className="wa-ui-tab-content">{selected ? <GroupBrowser key={accountIdentity} host={host} sessionId={selected} connected={connected} narrow={narrow} panelSide={panelSide} onOpenMessages={openGroup} /> : <NativeEmpty loading={list.loading} />}</TabsContent>
      <TabsContent value="connection" className="wa-ui-tab-content wa-ui-connection-scroll">{selected ? <SessionDetail key={`${selected}:${session?.created_at_ms ?? ''}`} host={host} sessionId={selected} refreshList={list.refresh} commands={commands} /> : <NativeEmpty loading={list.loading} />}</TabsContent>
      <TabsContent value="cloud" className="wa-ui-tab-content"><CloudPanel host={host} narrow={narrow} panelSide={panelSide} onDirty={onDirty} navigate={navigate} /></TabsContent>
    </Tabs>
    <StatusBar aria-label="WhatsApp status">{view === 'cloud' ? 'Meta Cloud API' : session ? `${session.session_id} · ${STATUS_LABEL[session.status]}` : 'No session selected'}<span>{view === 'messages' ? 'Session archive' : view === 'groups' ? 'Joined groups' : view === 'cloud' ? 'Business message archive' : 'Phone connection'}</span></StatusBar>
    {dialog}
  </PageShell>
}

function NativeEmpty({ loading }: { loading: boolean }) {
  return loading ? <Skeleton className="wa-ui-loading" /> : <EmptyState icon={MessageCircle} title="Connect a WhatsApp account" description="Create a session and link your phone to browse native messages and groups, or open Cloud API to use your Business Platform account." />
}
