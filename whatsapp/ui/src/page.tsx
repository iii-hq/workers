import { Button, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger, IconButton, Input, PageHeader, PageShell, Select, SettingsField, SettingsList, Skeleton, StatusBar, StatusPanel, Tabs, TabsContent, TabsList, TabsTrigger, useConfirm, type Host, type PageRenderProps } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Cable, Cloud, Ellipsis, MessageCircle, Plus, RefreshCw, Trash2, Users } from 'lucide-react'
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { RuntimeStatus } from './configuration'
import { CloudPanel } from './cloud'
import { GroupBrowser } from './groups'
import { DraftProvider, DraftScope, useDraftMemory, useDraftNavigation } from './drafts'
import type { ChatTarget } from './inbox-model'
import { MessageBrowser } from './messages'
import { requestScope, STATUS_LABEL, type Session } from './model'
import { SessionBadge, SessionDetail } from './session'

type Transport = 'native' | 'cloud'
type View = 'messages' | 'groups' | 'connection'

type WhatsAppPageProps = PageRenderProps & { host: Host; configurationId: string }

export function WhatsAppPage(props: WhatsAppPageProps) {
  return <DraftProvider><WhatsAppWorkspace {...props} /></DraftProvider>
}

function WhatsAppWorkspace({ host, configurationId, paneId, tabId, panelSide, onRequestClose, setDirty, commands }: WhatsAppPageProps) {
  const stateKey = `whatsapp:${configurationId}:${paneId || tabId || 'standalone'}`
  const [selected, setSelected] = usePaneState<string | null>(`${stateKey}:session`, null)
  const [newId, setNewId] = useState('')
  const [showCreate, setShowCreate] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [removing, setRemoving] = useState<string | null>(null)
  const [sessionMenuOpen, setSessionMenuOpen] = useState(false)
  const [removedSessions, setRemovedSessions] = useState<Set<string>>(() => new Set())
  const [removeFeedback, setRemoveFeedback] = useState<{ variant: 'success' | 'alert'; headline: string; detail: string } | null>(null)
  const [dirty, setDetailDirty] = useState(false)
  const [navigationError, setNavigationError] = useState<string | null>(null)
  const [destinationFocus, setDestinationFocus] = useState<{ current(): boolean; initiator?: HTMLElement | null } | null>(null)
  const drafts = useDraftMemory()
  const draftNavigation = useDraftNavigation()
  const [transport, setTransport] = useState<Transport>('native')
  const [view, setView] = useState<View>('connection')
  const [chat, setChat] = useState<ChatTarget | null>(null)
  const busy = useRef(false)
  const scope = useRef(requestScope())
  const { ref, narrow } = useContainerNarrow({ below: 760 })
  const shell = useRef<HTMLDivElement>(null)
  const shellRef = useCallback((node: HTMLDivElement | null) => { shell.current = node; ref(node) }, [ref])
  const { confirm, dialog } = useConfirm()
  const newIdRef = useRef<HTMLInputElement>(null)
  const newSessionRef = useRef<HTMLButtonElement>(null)
  const removeSessionRef = useRef<HTMLButtonElement>(null)
  const queuedSessionAction = useRef<(() => void) | null>(null)
  const sessionSelectRef = useRef<HTMLDivElement>(null)
  const focusAfterRemoval = useRef(false)
  const instance = useId()
  const list = useWorkerLive<Session[]>({
    iii: host.iii,
    triggers: [{ type: 'whatsapp::events', config: { kinds: ['session.created', 'session.connected', 'session.disconnected', 'session.stopped', 'session.starting', 'session.qr', 'session.logged_out'] } }],
    handlerId: `iii::whatsapp-ui::sessions::${instance}`, pollMs: 5000,
    fetch: () => host.iii.trigger<Session[]>('whatsapp::sessions::list'),
  })
  // A list request started before deletion may still return the old incarnation.
  const sessions = (list.data ?? []).filter((item) => !removedSessions.has(`${item.session_id}:${item.created_at_ms}`))
  const session = sessions.find((item) => item.session_id === selected)
  const connected = session?.status === 'connected' && !list.error
  const accountIdentity = `${selected}:${session?.created_at_ms ?? ''}:${session?.status === 'logged_out' ? 'logged-out' : 'active'}`

  useEffect(() => {
    if (!list.live) return
    const timer = window.setInterval(() => { if (!document.hidden) list.refresh() }, 10000)
    return () => window.clearInterval(timer)
  }, [list.live, list.refresh])
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
  }, [view, transport])
  useEffect(() => {
    if (!list.data || list.error || list.loading || sessions.some((item) => item.session_id === selected)) return
    const next = (sessions.find((item) => item.status === 'connected') ?? sessions[0])?.session_id ?? null
    if (next !== selected) { setSelected(next); setChat(null) }
  }, [list.data, list.error, list.loading, removedSessions, selected, setSelected])
  const onDirty = useCallback((next: boolean) => setDetailDirty(next), [])
  useEffect(() => { setDirty?.(dirty || drafts?.values.size ? 'WhatsApp message' : false); return () => setDirty?.(false) }, [setDirty, dirty, drafts?.values.size])
  useEffect(() => { if (showCreate) newIdRef.current?.focus() }, [showCreate])
  useEffect(() => {
    if (sessionMenuOpen || !queuedSessionAction.current) return
    const action = queuedSessionAction.current
    queuedSessionAction.current = null
    // Content unmounts on close; let its deferred trigger focus finish before the next surface opens.
    const timer = window.setTimeout(action, 0)
    return () => window.clearTimeout(timer)
  }, [sessionMenuOpen])
  useEffect(() => { if (!narrow || transport !== 'native') setSessionMenuOpen(false) }, [narrow, transport])
  useEffect(() => {
    if (focusAfterRemoval.current && !removing && removeFeedback) {
      const control = selected ? sessionSelectRef.current?.querySelector<HTMLElement>('button, select') : newIdRef.current
      if (control) { control.focus(); focusAfterRemoval.current = false }
    }
  }, [removing, removeFeedback, selected, list.loading])
  useEffect(() => {
    if (!destinationFocus) return
    // The shared dialog restores focus during its deferred cleanup. Run after
    // that cleanup and the destination commit, restoring the initiating control
    // when cancelled and the active destination when accepted.
    const timer = window.setTimeout(() => {
      if (!destinationFocus.current()) return
      const activeTabs = shell.current?.querySelectorAll<HTMLElement>('[role="tab"][data-state="active"]')
      const activeDestination = (showCreate || !selected) && transport === 'native'
        ? newIdRef.current
        : activeTabs?.item(activeTabs.length - 1)
      const initiator = destinationFocus.initiator
      // Radix tabs activate on keyboard focus. Focusing the rejected tab would
      // start the same navigation again immediately after cancellation.
      const inactiveTab = initiator?.getAttribute('role') === 'tab' && initiator.getAttribute('data-state') !== 'active'
      const activePanels = shell.current?.querySelectorAll<HTMLElement>('[role="tabpanel"][data-state="active"]')
      const editor = inactiveTab && Array.from(activePanels?.item(activePanels.length - 1)?.querySelectorAll<HTMLElement>('textarea:not([disabled]):not([readonly]), [role="textbox"][contenteditable="true"]') ?? [])
        .find((field) => !field.closest('[hidden], [aria-hidden="true"], details:not([open])'))
      const destination = initiator?.isConnected && !inactiveTab ? initiator : editor || activeDestination
      destination?.focus()
    }, 0)
    return () => window.clearTimeout(timer)
  }, [destinationFocus])
  useEffect(() => {
    if (session?.status === 'logged_out') { drafts?.clearPrefix(`native:${configurationId}:${selected}:${session.created_at_ms}:`); setChat(null); if (transport === 'native') setDetailDirty(false) }
  }, [session?.status, transport])

  const navigate = async (action: () => void): Promise<boolean> => {
    if (busy.current) return false
    if (drafts?.active?.busy) { setNavigationError('Wait for the current request to finish before switching. It will not be retried automatically.'); return false }
    setNavigationError(null)
    busy.current = true
    const current = scope.current.begin()
    try {
      const prompted = dirty
      const initiator = document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null
      const choice = prompted ? await draftNavigation.ask() : 'keep'
      if (choice !== 'edit' && current()) {
        if (choice === 'discard') drafts?.active?.clear()
        setDetailDirty(false); action()
        if (prompted) setDestinationFocus({ current })
        return true
      }
      if (prompted && current()) setDestinationFocus({ current, initiator })
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
    if (next === view && transport === 'native') return
    void navigate(() => { setTransport('native'); setView(next) })
  }
  const selectTransport = (next: Transport) => {
    if (next === transport) return
    void navigate(() => { setTransport(next); setShowCreate(false) })
  }
  const openCreate = () => { void navigate(() => { setTransport('native'); setShowCreate(true) }).then((accepted) => { if (!accepted) newSessionRef.current?.focus() }) }
  const cancelCreate = () => { setShowCreate(false); setCreateError(null); newSessionRef.current?.focus() }
  const runSessionMenuAction = (action: () => void) => { queuedSessionAction.current = action; setSessionMenuOpen(false) }
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
      setTransport('native')
      setChat(null)
      setView('connection')
      setNewId('')
      setShowCreate(false)
      list.refresh()
    } catch (cause) { if (current()) setCreateError(errorMessage(cause)) }
    finally { if (current()) { busy.current = false; setCreating(false) } }
  }
  const removeSession = async () => {
    if (busy.current || !session || list.error) return
    if (drafts?.active?.busy) { setNavigationError('Wait for the current request to finish before removing this session. It will not be retried automatically.'); return }
    const target = session
    busy.current = true
    const current = scope.current.begin()
    try {
      const accepted = await confirm({
        title: `Remove session "${target.session_id}"?`,
        description: 'This stops the session and permanently deletes its saved credentials and local message archive. You will need to link your phone again. It does not unlink the device on your phone.',
        details: dirty ? ['Your unsent draft will be discarded. A send already requested may still complete.'] : undefined,
        confirmLabel: 'Remove session', cancelLabel: 'Keep session', tone: 'danger',
      })
      if (!accepted || !current()) {
        if (current()) (narrow ? newSessionRef : removeSessionRef).current?.focus()
        return
      }
      focusAfterRemoval.current = true
      setRemoving(target.session_id)
      setRemoveFeedback(null)
      await host.iii.trigger<Session>('whatsapp::sessions::delete', { session_id: target.session_id }, { timeoutMs: 730000 })
      if (!current()) return
      drafts?.clearPrefix(`native:${configurationId}:${target.session_id}:${target.created_at_ms}:`)
      setRemovedSessions((previous) => new Set(previous).add(`${target.session_id}:${target.created_at_ms}`))
      const remaining = sessions.filter((item) => item.session_id !== target.session_id)
      setSelected((remaining.find((item) => item.status === 'connected') ?? remaining[0])?.session_id ?? null)
      setChat(null)
      setView('connection')
      setDetailDirty(false)
      setRemoveFeedback({ variant: 'success', headline: `Session "${target.session_id}" removed`, detail: 'Saved credentials and the local message archive were deleted.' })
      list.refresh()
    } catch (cause) {
      if (current()) {
        setRemoveFeedback({ variant: 'alert', headline: 'Session removal not confirmed', detail: `${errorMessage(cause)} Refresh the session list before trying again if the outcome is unclear.` })
        list.refresh()
      }
    } finally { if (current()) { busy.current = false; setRemoving(null) } }
  }
  useEffect(() => commands?.register([
    { id: 'refresh', title: 'Refresh sessions', shortcut: 'R', run: list.refresh },
    { id: 'new', title: 'Create session', shortcut: 'N', run: openCreate },
    { id: 'messages', title: 'Open messages', run: () => selectView('messages') },
    { id: 'groups', title: 'Open groups', run: () => selectView('groups') },
    { id: 'connection', title: 'Open connection', run: () => selectView('connection') },
    { id: 'cloud', title: 'Open Meta Cloud API', run: () => selectTransport('cloud') },
  ]), [commands, list.refresh, dirty, view, transport])

  const createForm = <form className="wa-ui-new-session" onSubmit={(event) => { event.preventDefault(); void create() }}>
    <SettingsList><SettingsField field="session_id" label="Session name" error={createError ?? undefined} description="Use 1–64 letters, numbers, hyphens or underscores to identify this phone connection." controlSize="full" layout="stacked" renderControl={(control) => <Input {...control} ref={newIdRef} value={newId} onChange={(next) => { setNewId(next); setCreateError(null) }} placeholder="e.g. support" maxLength={64} disabled={creating} autoComplete="off" spellCheck={false} />} /></SettingsList>
    <div className="wa-ui-create-actions"><Button type="submit" disabled={creating || !!removing || !newId.trim()}>{creating ? 'Creating…' : 'Create session'}</Button>{sessions.length > 0 && <Button variant="ghost" type="button" onClick={cancelCreate} disabled={creating}>Cancel</Button>}</div>
  </form>

  return <PageShell ref={shellRef} className="wa-ui-shell">
    <PageHeader className="wa-ui-navigation" icon={<MessageCircle size={16} />} title="WhatsApp" onClose={onRequestClose} />
    <div className="wa-ui-runtime"><RuntimeStatus host={host} quietWhenApplied /></div>
{navigationError && <div className="wa-ui-reader-notice"><StatusPanel role="alert" variant="alert" headline="Switching is temporarily unavailable" detail={navigationError} action={<Button variant="ghost" onClick={() => setNavigationError(null)}>Dismiss</Button>} /></div>}
    <Tabs value={transport} onValueChange={(value) => selectTransport(value as Transport)} className="wa-ui-tabs wa-ui-transport-tabs">
      <TabsList className="wa-ui-tab-list wa-ui-mode-list" aria-label="WhatsApp connection method"><TabsTrigger value="native" icon={<Cable size={16} />}>Linked phone</TabsTrigger><TabsTrigger value="cloud" icon={<Cloud size={16} />}>Meta Cloud API</TabsTrigger></TabsList>
      <TabsContent value="native" className="wa-ui-tab-content wa-ui-native-workspace">
        {sessions.length > 0 && <div className="wa-ui-session-toolbar" data-narrow={narrow || undefined} aria-label="Linked phone sessions">
          <div className="wa-ui-account-bar"><span className="wa-ui-session-label">Session</span><div ref={sessionSelectRef} className="wa-ui-session-select"><Select aria-label={`WhatsApp session: ${selected ?? 'Choose a session'}`} value={selected ?? undefined} options={sessions.map((item) => ({ value: item.session_id, label: item.session_id, description: STATUS_LABEL[item.status] }))} onChange={selectSession} placeholder="Choose a session" disabled={creating || !!removing} /></div>{session && view !== 'connection' && <span className="wa-ui-account-status"><SessionBadge status={session.status} /></span>}</div>
          <div className="wa-ui-session-actions">{narrow ? <DropdownMenu open={sessionMenuOpen} onOpenChange={setSessionMenuOpen}>
            <DropdownMenuTrigger asChild><IconButton ref={newSessionRef} label="Session actions" disabled={creating || !!removing}><Ellipsis size={16} /></IconButton></DropdownMenuTrigger>
            {sessionMenuOpen && <DropdownMenuContent align="end" className="wa-ui-session-menu">
              <DropdownMenuItem className="wa-ui-session-menu-item" onSelect={() => runSessionMenuAction(openCreate)} disabled={creating || !!removing}><Plus size={16} aria-hidden="true" />New session</DropdownMenuItem>
              <DropdownMenuItem className="wa-ui-session-menu-item" onSelect={() => runSessionMenuAction(list.refresh)} disabled={list.loading}><RefreshCw size={16} aria-hidden="true" />Refresh sessions</DropdownMenuItem>
              <DropdownMenuItem className="wa-ui-session-menu-item" onSelect={() => runSessionMenuAction(() => void removeSession())} disabled={!session || !!list.error || creating || !!removing}><Trash2 size={16} aria-hidden="true" />Remove session</DropdownMenuItem>
            </DropdownMenuContent>}
          </DropdownMenu> : <><Button ref={newSessionRef} variant="ghost" onClick={openCreate} disabled={creating || !!removing}><Plus size={16} aria-hidden="true" />New session</Button><IconButton label="Refresh sessions" onClick={list.refresh} disabled={list.loading}><RefreshCw size={16} /></IconButton><IconButton ref={removeSessionRef} label="Remove session" onClick={() => void removeSession()} disabled={!session || !!list.error || creating || !!removing}><Trash2 size={16} /></IconButton></>}</div>
        </div>}
        {list.error && <div className="wa-ui-reader-notice"><StatusPanel role="alert" variant="alert" headline="Could not refresh sessions" detail={list.error} action={<Button variant="ghost" onClick={list.refresh}>Try again</Button>} /></div>}
        {removeFeedback && <div className="wa-ui-reader-notice"><StatusPanel role={removeFeedback.variant === 'alert' ? 'alert' : 'status'} {...removeFeedback} action={<Button variant="ghost" onClick={() => setRemoveFeedback(null)}>Dismiss</Button>} /></div>}
        {removing ? <div className="wa-ui-reader-notice"><StatusPanel role="status" variant="info" headline={`Removing session "${removing}"`} detail="Stopping the session and deleting its local data…" /></div> : selected ? <>
          {showCreate && <section className="wa-ui-inline-create" aria-label="Create a linked phone session"><h2 className="wa-ui-title">New session</h2>{createForm}</section>}
          <DraftScope id={`native:${configurationId}:${accountIdentity}`}><Tabs value={view} onValueChange={(value) => selectView(value as View)} className="wa-ui-tabs">
            <TabsList className="wa-ui-tab-list wa-ui-native-tab-list" aria-label="Linked phone tools"><TabsTrigger value="connection" icon={<Cable size={16} />}>Connection</TabsTrigger><TabsTrigger value="messages" icon={<MessageCircle size={16} />}>Messages</TabsTrigger><TabsTrigger value="groups" icon={<Users size={16} />}>Groups</TabsTrigger></TabsList>
            <TabsContent value="connection" className="wa-ui-tab-content wa-ui-connection-scroll"><SessionDetail key={accountIdentity} host={host} sessionId={selected} refreshList={list.refresh} commands={commands} onDirty={onDirty} onOpenConversation={openGroup} /></TabsContent>
            <TabsContent value="messages" className="wa-ui-tab-content"><MessageBrowser key={accountIdentity} host={host} sessionId={selected} connected={connected} narrow={narrow} panelSide={panelSide} selected={chat} onSelect={selectChat} onDirty={onDirty} onGoConnection={() => selectView('connection')} /></TabsContent>
            <TabsContent value="groups" className="wa-ui-tab-content"><GroupBrowser key={accountIdentity} host={host} sessionId={selected} connected={connected} narrow={narrow} panelSide={panelSide} onOpenMessages={openGroup} /></TabsContent>
          </Tabs></DraftScope>
        </> : list.loading && !list.data ? <Skeleton className="wa-ui-loading" /> : !list.error && <div className="wa-ui-onboarding-layout"><section className="wa-ui-onboarding" aria-labelledby={`${instance}-phone-setup`}>
          <h2 id={`${instance}-phone-setup`} className="wa-ui-onboarding-title">Link a WhatsApp phone</h2>
          <p className="wa-ui-onboarding-copy">Connect WhatsApp on your phone to test messages and explore your groups.</p>
          <ol className="wa-ui-setup-steps"><li>Create a session for your phone.</li><li>Start it in Connection and scan the QR code in WhatsApp.</li><li>Send a test message, then browse Messages or Groups.</li></ol>
          {createForm}
        </section></div>}
      </TabsContent>
      <TabsContent value="cloud" className="wa-ui-tab-content"><CloudPanel host={host} configurationId={configurationId} narrow={narrow} panelSide={panelSide} onDirty={onDirty} navigate={navigate} /></TabsContent>
    </Tabs>
    <StatusBar className="wa-ui-status-bar" aria-label="WhatsApp status"><span className="wa-ui-status-identity" title={session?.session_id}>{transport === 'cloud' ? 'Meta Cloud API' : session ? narrow ? session.session_id : `Linked phone · ${session.session_id}${view !== 'connection' ? ` · ${STATUS_LABEL[session.status]}` : ''}` : 'Linked phone · No session'}</span><span>{transport === 'cloud' ? 'WhatsApp Business Platform' : view === 'messages' ? 'Session archive' : view === 'groups' ? 'Joined groups' : 'Phone connection'}</span></StatusBar>
    {dialog}
    {draftNavigation.dialog}
  </PageShell>
}
