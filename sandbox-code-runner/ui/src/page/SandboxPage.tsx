/**
 * The sandbox fleet page (#/ext/sandbox): the standard page chrome
 * (PageShell > PageHeader > PageBody side > PageSidebar + PageMain) over
 * the fleet rail and the selected sandbox's workspace — overview, console,
 * files behind Tabs. All data flows from the one push-driven store (useFleet);
 * chat cards cross-link in through src/lib/selection (a jump lands here
 * with the sandbox pre-selected, and selections made while this page is
 * mounted arrive over onSandboxSelected).
 */

import {
  Button,
  EmptyState,
  type Host,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  StatusDot,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { onSandboxSelected, takeSelectedSandbox } from '../lib/selection'
import { ConsoleTab } from './ConsoleTab'
import { CreateDialog } from './CreateDialog'
import { FilesTab } from './FilesTab'
import { FleetRail } from './FleetRail'
import { BoxIcon, PlayIcon, PlusIcon, RefreshIcon } from './icons'
import { OverviewTab } from './OverviewTab'
import { RunCodeDialog } from './RunCodeDialog'
import { useExecRecords } from './records'
import { useFleet } from './useFleet'

type TabId = 'overview' | 'console' | 'files'

const RAIL_WIDTH_DEFAULT = 264
const RAIL_WIDTH_MIN = 200
const RAIL_WIDTH_MAX = 420

export function SandboxPage({
  host,
  panelSide = 'left',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & PageRenderProps) {
  const { state, refresh, live } = useFleet(host)
  const [selected, setSelected] = useState<string | null>(null)
  // A cross-link can arrive before the first list lands — hold it until
  // the fleet can confirm the id, instead of selecting a ghost.
  const [pending, setPending] = useState<string | null>(() =>
    takeSelectedSandbox(),
  )
  const [tab, setTab] = useState<TabId>('overview')
  const [createOpen, setCreateOpen] = useState(false)
  const [runOpen, setRunOpen] = useState(false)

  // local display clock, not bus polling; the doctrine targets bus traffic
  // — change-only fleet events leave age_secs frozen, so displayed ages
  // and the reap countdown tick forward from snapshotAt on this clock.
  const [clock, setClock] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setClock(Date.now()), 1_000)
    return () => window.clearInterval(id)
  }, [])

  useEffect(() => onSandboxSelected((id) => setPending(id)), [])

  // A palette "sandboxes" row (or any other host.panels.open caller) selects
  // a sandbox by id through the standard panelContext channel; it rides the
  // same pending flow a chat cross-link uses.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context = panelContext.context
    const sandboxId =
      context && typeof context === 'object' && !Array.isArray(context)
        ? (context as Record<string, unknown>).sandboxId
        : null
    if (typeof sandboxId === 'string' && sandboxId) setPending(sandboxId)
  }, [panelContext])

  useEffect(() => {
    if (pending && state.sandboxes.some((s) => s.sandbox_id === pending)) {
      setSelected(pending)
      setPending(null)
      setTab('overview')
    }
  }, [pending, state.sandboxes])

  // Keep a useful default selection: adopt the first sandbox when nothing
  // is selected, and drop a selection whose sandbox left the fleet.
  useEffect(() => {
    if (selected && !state.sandboxes.some((s) => s.sandbox_id === selected)) {
      setSelected(null)
    }
    if (!selected && !pending && state.sandboxes.length > 0) {
      setSelected(state.sandboxes[0].sandbox_id)
    }
  }, [selected, pending, state.sandboxes])

  const sandbox = selected
    ? (state.sandboxes.find((s) => s.sandbox_id === selected) ?? null)
    : null
  const catalogImage = sandbox
    ? (state.images.find((img) => img.name === sandbox.image) ?? null)
    : null

  const timeline = useExecRecords(host, sandbox?.sandbox_id ?? null)

  const select = (id: string) => {
    setSelected(id)
    setTab('overview')
  }

  useEffect(
    () =>
      commands?.register([
        {
          id: 'new',
          title: 'New sandbox',
          detail: 'Create a sandbox from a catalog image',
          keywords: ['create', 'microvm'],
          shortcut: 'N',
          enabled: () => !state.daemonAbsent,
          run: () => setCreateOpen(true),
        },
        {
          id: 'run-code',
          title: 'Run code',
          detail: 'Run a snippet against a sandbox',
          keywords: ['exec', 'code'],
          shortcut: 'C',
          run: () => setRunOpen(true),
        },
        {
          id: 'refresh',
          title: 'Refresh the fleet',
          detail: 'Re-read sandboxes, runtimes and the catalog',
          keywords: ['reload'],
          shortcut: 'R',
          run: refresh,
        },
      ]),
    [commands, state.daemonAbsent, refresh],
  )

  function renderMain(): ReactNode {
    if (state.error && !state.daemonAbsent && state.sandboxes.length === 0) {
      return (
        <div className="cr-page-main-error" role="alert">
          <p>
            the fleet could not be read.
            <span className="cr-page-faint"> {state.error}</span>
          </p>
          <Button variant="ghost" size="sm" onClick={refresh}>
            retry
          </Button>
        </div>
      )
    }
    if (state.daemonAbsent) {
      return (
        <EmptyState
          icon={BoxIcon}
          title="no sandbox daemon on this engine"
          description="sandbox::list is not registered. Add the iii-sandbox daemon (`iii trigger compose::add worker=iii-sandbox`) and the fleet appears here — this page keeps checking."
        />
      )
    }
    if (sandbox === null) {
      if (state.sandboxes.length === 0 && !state.loading) {
        return (
          <EmptyState
            icon={BoxIcon}
            title="no sandboxes running"
            description="create a microVM from a catalog image, or let an agent's next sandbox::create populate the fleet."
            action={{
              label: 'create sandbox',
              onClick: () => setCreateOpen(true),
            }}
          />
        )
      }
      return (
        <EmptyState
          icon={BoxIcon}
          title="pick a sandbox"
          description="select a sandbox in the rail to inspect it, exec into it, or browse its files."
        />
      )
    }
    return (
      <Tabs
        value={tab}
        onValueChange={(next) => setTab(next as TabId)}
        className="cr-page-tabs"
      >
        <TabsList className="cr-page-tabs-list">
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="console">Console</TabsTrigger>
          <TabsTrigger value="files">Files</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="cr-page-tab-body">
          <OverviewTab
            key={sandbox.sandbox_id}
            host={host}
            sandbox={sandbox}
            snapshotAt={state.snapshotAt}
            clock={clock}
            catalogImage={catalogImage}
            onGoConsole={() => setTab('console')}
            onGoFiles={() => setTab('files')}
            onStopped={refresh}
          />
        </TabsContent>
        <TabsContent value="console" className="cr-page-tab-body">
          <ConsoleTab
            key={sandbox.sandbox_id}
            host={host}
            sandbox={sandbox}
            timeline={timeline}
          />
        </TabsContent>
        <TabsContent value="files" className="cr-page-tab-body">
          <FilesTab key={sandbox.sandbox_id} host={host} sandbox={sandbox} />
        </TabsContent>
      </Tabs>
    )
  }

  return (
    <PageShell className="cr-page-shell">
      <PageHeader
        icon={<BoxIcon size={16} />}
        title="Sandbox"
        description="MicroVM fleet · exec console · files"
        actions={
          <>
            <span
              className="cr-page-live"
              title={
                live
                  ? 'push only: fleet snapshots stream in on sandbox-code-runner::event'
                  : 'live binding unavailable — use refresh'
              }
            >
              <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
              {live ? 'live' : 'static'}
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={refresh}
              disabled={state.loading}
            >
              <RefreshIcon
                className={state.loading ? 'cr-page-spin' : undefined}
                aria-hidden
              />{' '}
              refresh
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setRunOpen(true)}>
              <PlayIcon aria-hidden /> run code
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={() => setCreateOpen(true)}
              disabled={state.daemonAbsent}
              data-autofocus=""
            >
              <PlusIcon aria-hidden /> new sandbox
            </Button>
          </>
        }
        onClose={onRequestClose}
      />

      {/* The `right` class mirrors `side` for the stylesheet (edge borders,
          narrow collapse) — PageBody itself only flips the flex order. */}
      <PageBody
        side={panelSide}
        className={`cr-page-body${panelSide === 'right' ? ' right' : ''}`}
      >
        <PageSidebar
          label="fleet"
          side={panelSide}
          collapsible
          resizable
          storageKey="sandbox:fleet"
          defaultWidth={RAIL_WIDTH_DEFAULT}
          minWidth={RAIL_WIDTH_MIN}
          maxWidth={RAIL_WIDTH_MAX}
          narrowBelow={700}
          className="cr-page-sidebar"
        >
          <FleetRail
            host={host}
            sandboxes={state.sandboxes}
            runtimes={state.runtimes}
            tombstones={state.tombstones}
            images={state.images}
            snapshotAt={state.snapshotAt}
            clock={clock}
            selected={selected}
            daemonAbsent={state.daemonAbsent}
            onSelect={select}
            onRefresh={refresh}
          />
        </PageSidebar>

        <PageMain className="cr-page-main">{renderMain()}</PageMain>
      </PageBody>

      <CreateDialog
        host={host}
        open={createOpen}
        images={state.images}
        onClose={() => setCreateOpen(false)}
        onCreated={(id) => {
          refresh()
          if (id) setPending(id)
        }}
      />
      <RunCodeDialog
        host={host}
        open={runOpen}
        runtimes={state.runtimes}
        onClose={() => setRunOpen(false)}
        onResult={(record, runSandboxId) => {
          // Only the timeline of the sandbox the run actually used gets the
          // record — a fresh-sandbox run belongs to no selected timeline.
          if (sandbox && runSandboxId === sandbox.sandbox_id) {
            timeline.append(record)
          }
          refresh()
        }}
      />
    </PageShell>
  )
}
