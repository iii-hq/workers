import {
  Button,
  EmptyState,
  type Host,
  List,
  ListItem,
  type LiveAnnouncement,
  LiveRegion,
  PageBody,
  PageHeader,
  type PageHeaderMenuItem,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Skeleton,
  StatusBar,
  StatusPanel,
  useConfirm,
} from '@iii-dev/console-ui'
import { copyText, errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, useCopyFlash, usePaneState } from '@iii-dev/console-ui/hooks'
import { ChevronLeft, Copy, FileJson, Plug, Plus, RefreshCw, X } from 'lucide-react'
import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createApi } from './api'
import { ConnectionView, Lines, ReviewPanel } from './ConnectionView'
import { currentRow, focusColumn, onColumnKeys } from './columns'
import {
  copyAsJson,
  type Draft,
  draftDirty,
  draftFromEntry,
  emptyDraft,
  entryFromDraft,
  entryRefs,
  sameJson,
} from './config-model'
import { FunctionDetail } from './FunctionDetail'
import { JsonView } from './JsonView'
import { ProcessView } from './ProcessView'
import {
  announcements,
  functionRows,
  type Level,
  narrowLevel,
  pageState,
  pathHeads,
  presentServer,
  type RowActionId,
  statusBarText,
  type Tone,
} from './presentation'
import { ServerContents } from './ServerContents'
import { ServerList } from './ServerList'
import { SignIn } from './SignIn'
import { approveEntry, removeEntry, type SaveDeps, saveBefore, saveEntry, setDisabled, shareEntry } from './save'
import type { Issue, Selection, Server, ServersList } from './types'
import { useServers } from './use-servers'

const SKELETON_ROWS = [0, 1, 2, 3]
const COLUMN_OF: Record<Level, number> = { servers: 1, contents: 2, detail: 3 }
const NO_ISSUES: Issue[] = []
const WORKER_DOWN = 'The mcp-client worker is not running'
const CONFIG_DOWN = 'The configuration worker cannot be reached'

/** The MCP servers page: the path row over servers, their contents and one thing in depth. */
export function McpServersPage({
  host,
  panelSide,
  onRequestClose,
  commands,
  setDirty,
}: { host: Host } & Partial<PageRenderProps>) {
  const data = useServers(host)
  const api = useMemo(() => createApi(host.iii), [host])
  const [selection, setSelection] = useState<Selection>({ server: null, item: null })
  const [level, setLevel] = useState<Level>('servers')
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  // The Connection form's draft: of the selected server, or of the server being added.
  const [draft, setDraft] = useState<Draft | null>(null)
  const [adding, setAdding] = useState(false)
  // What the open view reports: refused JSON text, or the Edit as JSON text.
  const [viewDirty, setViewDirty] = useState(false)
  // Edit as JSON's map as it opened: a refresh that cannot read the config leaves the view and its edits.
  const [jsonOpen, setJsonOpen] = useState<Record<string, unknown> | null>(null)
  const [errors, setErrors] = useState<Issue[]>(NO_ISSUES)
  const [saving, setSaving] = useState(false)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<{ headline: string; detail: string } | null>(null)
  // Raised by a row's Sign in; the Connection view's SignIn starts an attempt on the raise.
  const [startSignal, setStartSignal] = useState(0)
  const { confirm, dialog } = useConfirm()
  const [col1, setCol1] = usePaneState('mcp-client:col1', 288)
  const { ref: narrowRef, narrow } = useContainerNarrow()
  const root = useRef<HTMLDivElement | null>(null)
  const setRoot = useCallback(
    (el: HTMLDivElement | null) => {
      root.current = el
      narrowRef(el)
    },
    [narrowRef],
  )
  const state = pageState(data)
  // Adding shows the columns over a first run or a stopped worker.
  const shown = narrowLevel(adding ? 'ready' : state, level)
  const server = data.list?.servers.find((s) => s.name === selection.server) ?? null
  const heads = pathHeads(selection, server, data.list?.servers.length ?? Object.keys(data.config ?? {}).length)
  const { item } = selection
  const fnRow = useMemo(
    () =>
      server && item?.kind === 'function'
        ? functionRows(server, data.functions[server.name] ?? []).find((row) => row.functionId === item.functionId)
        : undefined,
    [server, item, data.functions],
  )
  const fnGone = item?.kind === 'function' && server !== null && fnRow === undefined
  const copyFlash = useCopyFlash(item?.kind === 'function' ? item.functionId : '')
  // A missing entry reads as `null`: `undefined` is the unsaved draft of Add server.
  const stored = adding || selection.server === null ? undefined : (data.config?.[selection.server] ?? null)
  // No import flow and no agent-access status yet: their buttons, commands and row stay hidden.
  const onImport = undefined as (() => void) | undefined
  const accessRow = null as { tone: Tone; text: string } | null
  // The entry the draft was loaded from or last saved as: an edit is measured against it, so a change made
  // outside the page is no edit, and dirty matches the draft the page holds whichever view is open.
  const [loaded, setLoaded] = useState<[string, unknown] | null>(null)
  const base = !adding && loaded && loaded[0] === selection.server ? loaded[1] : stored
  const formDirty = draft !== null && (adding || selection.server !== null) && draftDirty(draft, base)
  const dirty = viewDirty || formDirty
  // One save, approve, share, turn on or remove at a time, so two confirms never run at once.
  const working = busy || saving
  // Where the person is now: a save that resolves after they moved on leaves the form alone.
  const here = useRef({ server: selection.server, adding })
  here.current = { server: selection.server, adding }

  const deps = useMemo<SaveDeps>(
    () => ({
      api,
      // Save, Approve or Remove: the title's verb names the button.
      confirm: ({ title, lines, tone }) =>
        confirm({
          title,
          tone,
          confirmLabel: title.slice(0, title.indexOf(' ')),
          description: lines.length > 0 ? <Lines lines={lines} /> : undefined,
        }),
      // stub: the Agent access view replaces these
      removeBlock: async () => ({ ok: true, count: 0 }),
      blockCount: async () => 0,
    }),
    [api, confirm],
  )
  const announce = (text: string) => setAnnouncement((last) => ({ seq: (last?.seq ?? 0) + 1, text, urgency: 'polite' }))
  const confirmLeave = async () =>
    !dirty ||
    confirm({
      title: `Discard changes to ${jsonOpen ? 'the JSON' : draft?.name || 'the new server'}?`,
      confirmLabel: 'Discard',
    })
  const leave = useRef(confirmLeave)
  leave.current = confirmLeave

  const reconnect = useCallback(
    (name: string) => void api.reconnect(name).then(data.refresh, data.refresh),
    [api, data.refresh],
  )
  // Leaving a server, the new server or the JSON with unsaved edits asks first.
  const pickServer = async (next: Selection) => {
    const switching = jsonOpen !== null || (next.server !== null && (adding || next.server !== selection.server))
    if (switching) {
      if (!(await confirmLeave())) return false
      setJsonOpen(null)
      setErrors(NO_ISSUES)
      setFailure(null)
      if (next.server !== null) {
        const entry = data.config?.[next.server] ?? null
        setAdding(false)
        setLoaded([next.server, entry])
        setDraft(draftFromEntry(next.server, entry))
      }
    }
    setSelection(next)
    setLevel('contents')
    return true
  }
  const onAdd = useCallback(async () => {
    if (!(await leave.current())) return
    setJsonOpen(null)
    setErrors(NO_ISSUES)
    setFailure(null)
    setAdding(true)
    setDraft(emptyDraft())
    setSelection({ server: null, item: { kind: 'connection' } })
    setLevel('detail')
  }, [])
  useEffect(() => setDirty?.(dirty ? 'MCP server' : false), [dirty, setDirty])

  const onSave = async () => {
    if (!draft || working) return
    const { name } = draft
    if (adding && data.config && name in data.config) {
      setErrors([{ path: `/mcpServers/${name}`, message: `A server named \`${name}\` already exists.` }])
      return
    }
    const entry = entryFromDraft(draft)
    // The view the save started from: the add, or the selected server.
    const stillHere = () =>
      adding ? here.current.adding : !here.current.adding && here.current.server === selection.server
    const fail = (detail: string) =>
      stillHere() ? setFailure({ headline: 'Could not save', detail }) : announce(`${name} was not saved: ${detail}`)
    setSaving(true)
    setFailure(null)
    try {
      const result = await saveEntry(deps, name, entry, saveBefore(adding, name, data.config, data.list?.servers))
      if (result.message) fail(result.message)
      if (result.saved) {
        data.refresh()
        announce(`${name} saved`)
      }
      if (!stillHere()) return
      setErrors(result.errors)
      if (!result.saved) return
      // What was saved is the new base: a draft still equal to it is clean, one edited meanwhile stays dirty.
      setLoaded([name, entry])
      setAdding(false)
      setSelection({ server: name, item: { kind: 'connection' } })
    } catch (err) {
      fail(errorMessage(err))
    } finally {
      setSaving(false)
    }
  }
  const onDiscard = () => {
    setErrors(NO_ISSUES)
    setFailure(null)
    if (selection.server !== null) {
      setLoaded([selection.server, stored])
      setDraft(draftFromEntry(selection.server, stored))
    } else {
      setAdding(false)
      setDraft(null)
      setSelection({ server: null, item: null })
      setLevel('servers')
    }
  }
  // Approve, Share, Turn on and Remove: one at a time, a failure shown above the form.
  const attempt = async (headline: string, run: () => Promise<boolean | undefined>) => {
    if (working) return
    setBusy(true)
    setFailure(null)
    try {
      if ((await run()) !== false) data.refresh()
    } catch (err) {
      setFailure({ headline, detail: errorMessage(err) })
    } finally {
      setBusy(false)
    }
  }
  const approve = (s: Server) => attempt('Could not approve', () => approveEntry(deps, s))
  const turnOn = (s: Server) =>
    attempt('Could not turn on', async () => {
      await setDisabled(api, s.name, false)
      return true
    })
  const share = (s: Server) =>
    attempt('Could not share', () => shareEntry(deps, s.name, entryRefs(data.config?.[s.name])))
  const remove = (s: Server) =>
    attempt('Could not remove', async () => {
      if (!(await removeEntry(deps, s))) return false
      setSelection({ server: null, item: null })
      setDraft(null)
      setLevel('servers')
      announce(`${s.name} removed`)
      return true
    })
  const openJson = async () => {
    if (!data.config || !(await confirmLeave())) return
    setAdding(false)
    setDraft(null)
    setErrors(NO_ISSUES)
    setFailure(null)
    setSelection({ server: null, item: null })
    setJsonOpen(data.config)
    setLevel('contents')
  }
  const closeJson = () => {
    setJsonOpen(null)
    setLevel('servers')
    data.refresh()
  }
  const copyJson = () => {
    if (data.config)
      void copyText(copyAsJson(data.config)).then((ok) => announce(ok ? 'Config copied as JSON' : 'Could not copy'))
  }
  const onDirty = useCallback((next: boolean) => setViewDirty(next), [])

  // A refresh that changed the entry reaches the form, unless the form holds an edit. Every fetch builds a new
  // config, so the content is compared: an unchanged refresh keeps the draft and its rows not yet filled in.
  // A missing entry is not loaded: a server just added is not in the config until the refresh lands.
  useEffect(() => {
    if (adding || dirty || selection.server === null || stored === null || sameJson(loaded, [selection.server, stored]))
      return
    setLoaded([selection.server, stored])
    setDraft(draftFromEntry(selection.server, stored))
  }, [adding, dirty, selection.server, stored, loaded])
  const pickItem = (next: Selection) => {
    setSelection(next)
    setLevel('detail')
  }
  const onAction = async (name: string, action: RowActionId) => {
    if (!(await pickServer({ server: name, item: { kind: 'connection' } }))) return
    const s = data.list?.servers.find((row) => row.name === name)
    if (action === 'retry' || action === 'restart') reconnect(name)
    else if (s && action === 'review') void approve(s)
    else if (s && action === 'turn_on') void turnOn(s)
    else if (s && action === 'share') void share(s)
    else if (action === 'sign_in') setStartSignal((n) => n + 1)
  }
  // A raise is delivered once: a SignIn mounted later never reads it as a new one.
  useEffect(() => {
    if (startSignal > 0) setStartSignal(0)
  }, [startSignal])
  const back = (to: Level, label: string) =>
    narrow ? (
      <Button
        variant="ghost"
        size="sm"
        className="mcp-ui-back"
        aria-label={`Back to ${label}`}
        onClick={() => setLevel(to)}
      >
        <ChevronLeft size={16} aria-hidden />
        {label}
      </Button>
    ) : null

  // A function the server no longer lists leaves its server on Connection.
  useEffect(() => {
    if (fnGone) setSelection((s) => ({ server: s.server, item: { kind: 'connection' } }))
  }, [fnGone])

  // Each server that became ready since the last read is said once.
  const previousList = useRef<ServersList | null>(null)
  useEffect(() => {
    if (!data.list) return
    const texts = announcements(previousList.current, data.list)
    previousList.current = data.list
    if (texts.length === 0) return
    setAnnouncement((last) => ({ seq: (last?.seq ?? 0) + 1, text: texts.join('. '), urgency: 'polite' }))
  }, [data.list])

  // A narrow drill-in hides the column that held focus: focus moves into the one shown.
  const lastShown = useRef(shown)
  useEffect(() => {
    if (lastShown.current === shown) return
    lastShown.current = shown
    if (narrow) focusColumn(root.current, COLUMN_OF[shown])
  }, [shown, narrow])

  useEffect(
    () =>
      commands?.register([
        { id: 'add-server', title: 'Add MCP server', enabled: () => onAdd !== undefined, run: () => onAdd?.() },
        {
          id: 'import-config',
          title: 'Import MCP config',
          enabled: () => onImport !== undefined,
          run: () => onImport?.(),
        },
        {
          id: 'reconnect',
          title: 'Reconnect server',
          enabled: () => selection.server !== null,
          run: () => {
            if (selection.server !== null) reconnect(selection.server)
          },
        },
        {
          id: 'copy-id',
          title: 'Copy function id',
          enabled: () => selection.item?.kind === 'function',
          run: copyFlash.copy,
        },
        {
          id: 'review-access',
          title: 'Review agent access',
          enabled: () => accessRow !== null,
          run: () => {
            setSelection({ server: selection.server, item: { kind: 'access' } })
            setLevel('detail')
          },
        },
      ]),
    [commands, selection, reconnect, copyFlash.copy, onAdd, onImport, accessRow],
  )
  const retry = (
    <Button variant="secondary" size="sm" onClick={data.refresh}>
      Retry
    </Button>
  )
  const menu: PageHeaderMenuItem[] = [
    {
      id: 'edit-json',
      label: 'Edit as JSON',
      icon: <FileJson size={16} />,
      disabled: !data.config || jsonOpen !== null,
      onSelect: () => void openJson(),
    },
    { id: 'copy-json', label: 'Copy as JSON', icon: <Copy size={16} />, disabled: !data.config, onSelect: copyJson },
    { id: 'refresh', label: 'Refresh', icon: <RefreshCw size={16} />, onSelect: data.refresh },
  ]
  const bar = dirty || saving
  const cwdOf = (name: string) => {
    const cwd = (data.config?.[name] as { cwd?: unknown } | null | undefined)?.cwd
    return typeof cwd === 'string' ? cwd : null
  }
  const panel = server ? presentServer(server, 0).panel : null

  return (
    <div
      className="mcp-ui-root"
      ref={setRoot}
      data-narrow={narrow}
      data-level={shown}
      data-side={panelSide}
      style={{ '--mcp-ui-col1': `${col1}px` } as CSSProperties}
    >
      <PageShell>
        <PageHeader
          icon={<Plug size={16} />}
          title="MCP servers"
          description="Tools from MCP servers, registered as iii functions"
          actions={
            <Button variant="secondary" size="sm" onClick={onAdd}>
              <Plus size={16} aria-hidden />
              Add server
            </Button>
          }
          menu={menu}
          onClose={onRequestClose}
        />
        <div className="mcp-ui-path">
          <div className="mcp-ui-path-head">
            <code>{heads.first}</code>
            {state === 'loading' ? null : <span>{heads.firstMeta}</span>}
          </div>
          <div className="mcp-ui-path-head">
            {jsonOpen ? <b>Edit as JSON</b> : null}
            {heads.second ? <code>{heads.second}</code> : null}
            {heads.secondMeta ? <span>{heads.secondMeta}</span> : null}
          </div>
          <div className="mcp-ui-path-head">
            {jsonOpen ? (
              <Button variant="ghost" size="sm" onClick={async () => (await confirmLeave()) && closeJson()}>
                <X size={16} aria-hidden />
                Close
              </Button>
            ) : null}
            {heads.thirdIsId ? <code>{heads.third}</code> : heads.third ? <b>{heads.third}</b> : null}
            {heads.thirdIsId ? (
              <Button variant="ghost" size="sm" onClick={copyFlash.copy}>
                <Copy size={16} aria-hidden />
                {copyFlash.state === 'copied' ? 'Copied' : 'Copy function id'}
              </Button>
            ) : null}
          </div>
        </div>
        <PageBody side={panelSide}>
          {state === 'config_down' && !jsonOpen ? (
            <PageMain className="mcp-ui-whole">
              <StatusPanel variant="alert" headline={CONFIG_DOWN} detail={data.configError} action={retry} />
            </PageMain>
          ) : (
            <>
              <PageSidebar
                className="mcp-ui-col1"
                label="Servers"
                side={panelSide}
                resizable
                minWidth={240}
                maxWidth={420}
                width={col1}
                onWidthChange={setCol1}
                narrow={narrow}
              >
                <ServerList
                  data={data}
                  selection={selection}
                  onSelect={pickServer}
                  onAction={onAction}
                  onAdd={onAdd}
                  onImport={onImport}
                  draft={adding && draft ? draft : null}
                />
              </PageSidebar>
              <PageMain className="mcp-ui-main">
                {jsonOpen ? (
                  <JsonView
                    deps={deps}
                    mcpServers={jsonOpen}
                    onClose={closeJson}
                    setDirty={onDirty}
                    blocked={data.workerDown ? WORKER_DOWN : data.configDown ? CONFIG_DOWN : null}
                  />
                ) : state === 'worker_down' && !adding ? (
                  <div className="mcp-ui-whole">
                    <StatusPanel
                      variant="alert"
                      headline="The mcp-client worker is not running"
                      detail="Start it, then retry. Servers and their functions come back by themselves."
                      action={retry}
                    />
                  </div>
                ) : state === 'first_run' && !adding ? (
                  <div className="mcp-ui-whole">
                    <EmptyState
                      icon={Plug}
                      title="No MCP servers yet"
                      description="Add a server to register its tools as iii functions."
                      action={{ label: 'Add server', onClick: onAdd }}
                    />
                  </div>
                ) : (
                  <div className="mcp-ui-cols">
                    <div className="mcp-ui-col2">
                      {state === 'loading' ? (
                        <>
                          <div className="mcp-ui-search" aria-hidden>
                            <Skeleton className="mcp-ui-skeleton-search" />
                          </div>
                          <div className="mcp-ui-list" aria-hidden>
                            {SKELETON_ROWS.map((n) => (
                              <ListItem
                                key={n}
                                as="div"
                                className="mcp-ui-fn"
                                label={<Skeleton className="mcp-ui-skeleton-name" />}
                                description={<Skeleton className="mcp-ui-skeleton-text" />}
                              />
                            ))}
                          </div>
                        </>
                      ) : (
                        <>
                          {back('servers', 'MCP servers')}
                          {adding ? (
                            <List
                              role="group"
                              aria-label="New server contents"
                              className="mcp-ui-list"
                              data-column="2"
                              onKeyDown={onColumnKeys}
                            >
                              <ListItem
                                className="mcp-ui-pin"
                                {...currentRow(true)}
                                label="Connection"
                                onClick={() => pickItem({ server: null, item: { kind: 'connection' } })}
                              />
                            </List>
                          ) : server ? (
                            <ServerContents
                              key={server.name}
                              server={server}
                              functions={data.functions[server.name] ?? []}
                              selection={selection}
                              onSelect={pickItem}
                              accessRow={accessRow}
                              onRetryLists={() => reconnect(server.name)}
                            />
                          ) : null}
                        </>
                      )}
                    </div>
                    <div className="mcp-ui-col3" data-column="3">
                      {state === 'loading' ? (
                        SKELETON_ROWS.map((n) => <Skeleton key={n} className="mcp-ui-skeleton-line" />)
                      ) : (
                        <>
                          {selection.server ? back('contents', selection.server) : null}
                          {adding ? back('contents', 'New server') : null}
                          {(server || adding || selection.server) && item?.kind === 'connection' && draft ? (
                            <ConnectionView
                              key={adding ? '' : selection.server}
                              api={api}
                              host={host}
                              server={server}
                              draft={draft}
                              onDraft={setDraft}
                              errors={errors}
                              setDirty={onDirty}
                              stored={base}
                              onSave={() => void onSave()}
                              onDiscard={onDiscard}
                              onRemove={server && !adding ? () => void remove(server) : null}
                              saving={saving}
                              blocked={data.workerDown ? WORKER_DOWN : null}
                            >
                              {failure ? (
                                <StatusPanel
                                  variant="alert"
                                  role="alert"
                                  headline={failure.headline}
                                  detail={failure.detail}
                                />
                              ) : null}
                              {server && !adding && server.error?.code === 'secret_not_shared' && panel ? (
                                <StatusPanel
                                  variant="alert"
                                  headline={panel.headline}
                                  detail={panel.next}
                                  action={
                                    <Button
                                      size="sm"
                                      variant={bar ? 'secondary' : 'primary'}
                                      disabled={working}
                                      onClick={() => void share(server)}
                                    >
                                      Share
                                    </Button>
                                  }
                                />
                              ) : null}
                              {server && !adding && server.state === 'unapproved' ? (
                                <ReviewPanel
                                  api={api}
                                  server={server}
                                  entry={data.config?.[server.name]}
                                  busy={working}
                                  quiet={bar}
                                  onApprove={() => void approve(server)}
                                />
                              ) : null}
                              {server && !adding && server.state === 'disabled' ? (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  disabled={working}
                                  onClick={() => void turnOn(server)}
                                >
                                  Turn on
                                </Button>
                              ) : null}
                              {server && !adding ? (
                                <SignIn api={api} server={server} startSignal={startSignal} quiet={bar} />
                              ) : null}
                            </ConnectionView>
                          ) : null}
                          {server && item?.kind === 'process' ? (
                            <ProcessView key={server.name} api={api} server={server} cwd={cwdOf(server.name)} />
                          ) : null}
                          {server && fnRow ? (
                            <FunctionDetail api={api} server={server} row={fnRow} host={host} commands={commands} />
                          ) : null}
                        </>
                      )}
                    </div>
                  </div>
                )}
              </PageMain>
            </>
          )}
        </PageBody>
        {data.list ? (
          <StatusBar
            className="mcp-ui-status"
            end={narrow ? undefined : 'Arrow keys move between columns · Mod+Enter runs the test'}
          >
            {statusBarText(data.list)}
          </StatusBar>
        ) : null}
      </PageShell>
      <LiveRegion announcement={announcement} />
      {dialog}
    </div>
  )
}
