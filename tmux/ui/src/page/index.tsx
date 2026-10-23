/**
 * The tmux page (a Terminal/instrument archetype): the session tree on the
 * side, one live tmux client in the main column, and the machine's
 * listening ports — `iii compose` stacks first — one tab away. A sub-agent
 * picked in the tree opens its transcript in a third tab.
 *
 * The page's terminal is an ordinary tmux client. Choosing a window of the
 * session it is on runs `select-window`, so every client of that session —
 * the operator's own terminal included — follows, exactly as in tmux.
 * Choosing anything in another session moves only this client there.
 */

import {
  Button,
  type Host,
  IconButton,
  LiveRegion,
  type LiveAnnouncement,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import { Bot, Network, Plus, RefreshCw, SquareTerminal } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createApi } from '../lib/api'
import { useTmuxOverview } from '../lib/live'
import {
  activeWindow,
  buildSessionTree,
  compactRequest,
  findSession,
  pickInitialSession,
  type SessionRow,
  type SubagentRow,
  type WindowRow,
  windowLabel,
} from '../lib/model'
import type { OutputRouter } from '../lib/output-router'
import type { PaneRef } from '../lib/types'
import { NewSessionDialog, NewWindowDialog, RenameDialog } from './dialogs'
import { PortsView } from './PortsView'
import { SessionTree } from './SessionTree'
import { SubagentView } from './SubagentView'
import { TerminalView } from './TerminalView'
import { useTmuxTerminal } from './use-terminal'

type DialogState =
  | { kind: 'new-session' }
  | { kind: 'new-window'; session: SessionRow }
  | { kind: 'rename-session'; session: SessionRow }
  | { kind: 'rename-window'; session: SessionRow; window: WindowRow }
  | null

type View = 'terminal' | 'ports' | 'subagent'

export function TmuxPage({
  host,
  router,
  panelSide,
  tabId,
  paneId,
  onRequestClose,
  commands,
}: PageRenderProps & { host: Host; router: OutputRouter }) {
  const api = useMemo(() => createApi(host.iii), [host])
  const overview = useTmuxOverview(host.iii, api)
  const { ref: narrowRef, narrow } = useContainerNarrow({ below: 640 })
  const paneKey = paneId || tabId || 'default'
  const [remembered, setRemembered] = usePaneState<string | null>(`iii::tmux-ui::session::${paneKey}`, null)
  const [view, setView] = usePaneState<View>(`iii::tmux-ui::view::${paneKey}`, 'terminal')
  const [dialog, setDialog] = useState<DialogState>(null)
  // The sub-agent shown in the third tab; kept after it finishes and leaves the tree.
  const [subagent, setSubagent] = useState<SubagentRow | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const announceSeq = useRef(0)
  const { confirm, dialog: confirmDialog } = useConfirm()

  const announce = useCallback((text: string) => {
    announceSeq.current += 1
    setAnnouncement({ seq: announceSeq.current, text, urgency: 'polite' })
  }, [])

  // Where the terminal opens first; later moves go through moveTo.
  const initial = pickInitialSession(overview.sessions, remembered)
  const initialId = initial?.id ?? null
  const target = useMemo(() => (initialId ? { session: initialId } : null), [initialId])
  const terminal = useTmuxTerminal({ host, api, router, target })

  const currentSessionId = terminal.sessionId ?? initialId
  const currentSession = findSession(overview.sessions, currentSessionId)
  const currentWindow = activeWindow(currentSession)
  const rows = useMemo(
    () => buildSessionTree(overview.sessions, overview.ports, currentSessionId),
    [overview.sessions, overview.ports, currentSessionId],
  )

  // Remember the session by name: ids restart with the tmux server.
  useEffect(() => {
    if (currentSession && currentSession.name !== remembered) setRemembered(currentSession.name)
  }, [currentSession, remembered, setRemembered])

  const fail = useCallback(
    (what: string, error: unknown) => {
      const text = `${what}: ${errorMessage(error)}`
      setNotice(text)
      announce(text)
    },
    [announce],
  )

  const showTerminal = useCallback(() => {
    setView('terminal')
    window.requestAnimationFrame(() => terminal.focus())
  }, [setView, terminal])

  const selectWindow = useCallback(
    (sessionId: string, windowId: string) => {
      showTerminal()
      if (terminal.status === 'ready' && terminal.sessionId === sessionId) {
        api.selectWindow(sessionId, windowId).then(overview.refresh, (error) => fail('Could not select the window', error))
        return
      }
      terminal.moveTo({ session: sessionId, window: windowId })
    },
    [api, fail, overview.refresh, showTerminal, terminal],
  )

  const selectSession = useCallback(
    (row: SessionRow) => {
      showTerminal()
      if (terminal.status === 'ready' && terminal.sessionId === row.session.id) return
      terminal.moveTo({ session: row.session.id })
    },
    [showTerminal, terminal],
  )

  const selectSubagent = useCallback(
    (sub: SubagentRow) => {
      setSubagent(sub)
      setView('subagent')
    },
    [setView],
  )

  const closeSubagent = useCallback(() => {
    setSubagent(null)
    showTerminal()
  }, [showTerminal])

  const selectLocation = useCallback(
    (location: PaneRef) => selectWindow(location.session_id, location.window_id),
    [selectWindow],
  )

  const reconnect = useCallback(() => {
    const session = currentSession ?? pickInitialSession(overview.sessions, null)
    terminal.reconnect(session ? { session: session.id } : undefined)
  }, [currentSession, overview.sessions, terminal])

  const killSession = useCallback(
    async (row: SessionRow) => {
      const ok = await confirm({
        title: `Kill session ${row.session.name}?`,
        description:
          'Every window and process in it ends, and every terminal attached to it — yours included — detaches.',
        details: row.windows.map((w) => `${w.label} — ${w.agents[0]?.label ?? w.command}`),
        confirmLabel: 'Kill session',
        tone: 'danger',
      })
      if (!ok) return
      try {
        await api.killSession(row.session.id)
        announce(`Session ${row.session.name} killed`)
        overview.refresh()
      } catch (error) {
        fail(`Could not kill ${row.session.name}`, error)
      }
    },
    [announce, api, confirm, fail, overview],
  )

  const killWindow = useCallback(
    async (session: SessionRow, row: WindowRow) => {
      const ok = await confirm({
        title: `Kill window ${row.label}?`,
        description: `Every process in ${session.session.name}:${row.label} ends.`,
        confirmLabel: 'Kill window',
        tone: 'danger',
      })
      if (!ok) return
      try {
        await api.killWindow(session.session.id, row.window.id)
        announce(`Window ${row.label} killed`)
        overview.refresh()
      } catch (error) {
        fail(`Could not kill ${row.label}`, error)
      }
    },
    [announce, api, confirm, fail, overview],
  )

  const recentDirectories = useMemo(() => host.workspace?.recentDirectories() ?? [], [host])
  const tmux = overview.sessions?.tmux ?? null
  const noSessions = overview.sessions !== null && overview.sessions.sessions.length === 0

  const empty = useMemo(() => {
    if (tmux && !tmux.available) {
      return {
        title: 'tmux is not available',
        description: tmux.error ?? 'Install tmux on the machine running the tmux worker.',
      }
    }
    if (noSessions) {
      return {
        title: 'No tmux sessions',
        description: 'Start one here, or run tmux in any terminal — it shows up at once.',
        action: { label: 'New session', onClick: () => setDialog({ kind: 'new-session' }) },
      }
    }
    return null
  }, [noSessions, tmux])

  // Palette rows and pane-scoped commands while the page is mounted.
  useEffect(() => {
    if (!commands) return
    return commands.register([
      { id: 'new-session', title: 'New tmux session', run: () => setDialog({ kind: 'new-session' }) },
      { id: 'show-ports', title: 'Show listening ports', run: () => setView('ports') },
      { id: 'show-terminal', title: 'Show the tmux terminal', run: showTerminal },
      { id: 'refresh', title: 'Refresh tmux sessions and ports', run: overview.refresh },
    ])
  }, [commands, overview.refresh, setView, showTerminal])

  const description = tmux
    ? tmux.available
      ? [tmux.version ? `tmux ${tmux.version}` : 'tmux', tmux.socket_path].filter(Boolean).join(' · ')
      : 'tmux not found'
    : 'Sessions, windows and the ports behind them'

  const newSessionButton = (
    <Button size="sm" variant="ghost" onClick={() => setDialog({ kind: 'new-session' })}>
      <Plus aria-hidden />
      New session
    </Button>
  )

  return (
    <PageShell className="tmux-shell" ref={narrowRef}>
      {confirmDialog}
      <LiveRegion announcement={announcement} />
      <PageHeader
        icon={<SquareTerminal />}
        title="tmux"
        description={<span className="tmux-mono">{description}</span>}
        actions={
          <IconButton label="Refresh sessions and ports" variant="ghost" onClick={overview.refresh}>
            <RefreshCw aria-hidden />
          </IconButton>
        }
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageSidebar
          label="tmux sessions"
          side={panelSide}
          collapsible
          resizable
          storageKey="tmux:sessions"
          defaultWidth={300}
          minWidth={220}
          maxWidth={480}
          narrowBelow={640}
          narrowMode="drawer"
          collapsedActions={
            <IconButton label="New session" onClick={() => setDialog({ kind: 'new-session' })}>
              <Plus aria-hidden />
            </IconButton>
          }
        >
          <div className="tmux-side">
            <div className="tmux-side-top">{newSessionButton}</div>
            <div className="tmux-side-scroll">
              {overview.error && !overview.sessions ? (
                <StatusPanel
                  variant="alert"
                  headline="Could not list tmux sessions"
                  detail={overview.error}
                  action={
                    <Button size="sm" variant="ghost" onClick={overview.refresh}>
                      Retry
                    </Button>
                  }
                />
              ) : overview.loading && !overview.sessions ? (
                <p className="tmux-faint tmux-side-note">Reading tmux…</p>
              ) : rows.length === 0 ? (
                <p className="tmux-faint tmux-side-note">No sessions on this tmux server.</p>
              ) : (
                <SessionTree
                  rows={rows}
                  narrow={narrow}
                  onSelectSession={selectSession}
                  onSelectWindow={(session, row) => selectWindow(session.session.id, row.window.id)}
                  selectedSubagent={view === 'subagent' ? (subagent?.key ?? null) : null}
                  onSelectSubagent={selectSubagent}
                  onNewWindow={(session) => setDialog({ kind: 'new-window', session })}
                  onRenameSession={(session) => setDialog({ kind: 'rename-session', session })}
                  onKillSession={(session) => void killSession(session)}
                  onRenameWindow={(session, window) => setDialog({ kind: 'rename-window', session, window })}
                  onKillWindow={(session, row) => void killWindow(session, row)}
                />
              )}
            </div>
          </div>
        </PageSidebar>
        <PageMain className="tmux-main">
          {notice ? (
            <StatusPanel
              variant="alert"
              headline="tmux refused that"
              detail={notice}
              action={
                <Button size="sm" variant="ghost" onClick={() => setNotice(null)}>
                  Dismiss
                </Button>
              }
            />
          ) : null}
          <Tabs
            className="tmux-tabs"
            value={view === 'subagent' && !subagent ? 'terminal' : view}
            onValueChange={(next) => setView(next as View)}
          >
            <TabsList variant="line" aria-label="tmux views">
              <TabsTrigger value="terminal" icon={<SquareTerminal aria-hidden />}>
                Terminal
              </TabsTrigger>
              <TabsTrigger value="ports" icon={<Network aria-hidden />}>
                Ports
              </TabsTrigger>
              {subagent ? (
                <TabsTrigger value="subagent" icon={<Bot aria-hidden />}>
                  Sub-agent
                </TabsTrigger>
              ) : null}
            </TabsList>
            <TabsContent value="terminal" forceMount className="tmux-tab">
              <TerminalView
                terminal={terminal}
                sessionName={currentSession?.name ?? null}
                windowLabel={currentWindow ? windowLabel(currentWindow) : null}
                live={overview.live}
                empty={empty}
                onReconnect={reconnect}
              />
            </TabsContent>
            <TabsContent value="ports" className="tmux-tab tmux-tab-scroll">
              <PortsView ports={overview.ports} loading={overview.loading} onSelectLocation={selectLocation} />
            </TabsContent>
            {subagent ? (
              <TabsContent value="subagent" className="tmux-tab">
                <SubagentView api={api} subagent={subagent} onClose={closeSubagent} />
              </TabsContent>
            ) : null}
          </Tabs>
        </PageMain>
      </PageBody>

      {dialog?.kind === 'new-session' ? (
        <NewSessionDialog
          recentDirectories={recentDirectories}
          onClose={() => setDialog(null)}
          onSubmit={async (values) => {
            const created = await api.createSession(compactRequest({ ...values }))
            announce(`Session ${created.session} created`)
            showTerminal()
            terminal.moveTo({ session: created.session_id })
            overview.refresh()
          }}
        />
      ) : null}
      {dialog?.kind === 'new-window' ? (
        <NewWindowDialog
          sessionName={dialog.session.session.name}
          recentDirectories={recentDirectories}
          onClose={() => setDialog(null)}
          onSubmit={async (values) => {
            const sessionId = dialog.session.session.id
            const created = await api.createWindow(
              compactRequest({
                session: sessionId,
                name: values.name,
                cwd: values.cwd,
                command: values.command,
                select: values.select,
              }),
            )
            announce(`Window ${created.window_index} created in ${dialog.session.session.name}`)
            if (values.select && terminal.sessionId !== sessionId) {
              showTerminal()
              terminal.moveTo({ session: sessionId, window: created.window_id })
            }
            overview.refresh()
          }}
        />
      ) : null}
      {dialog?.kind === 'rename-session' ? (
        <RenameDialog
          title={`Rename session ${dialog.session.session.name}`}
          current={dialog.session.session.name}
          onClose={() => setDialog(null)}
          onSubmit={async (name) => {
            await api.renameSession(dialog.session.session.id, name)
            announce(`Session renamed to ${name}`)
            overview.refresh()
          }}
        />
      ) : null}
      {dialog?.kind === 'rename-window' ? (
        <RenameDialog
          title={`Rename window ${dialog.window.label}`}
          current={dialog.window.window.name}
          onClose={() => setDialog(null)}
          onSubmit={async (name) => {
            await api.renameWindow(dialog.session.session.id, dialog.window.window.id, name)
            announce(`Window renamed to ${name}`)
            overview.refresh()
          }}
        />
      ) : null}
    </PageShell>
  )
}
