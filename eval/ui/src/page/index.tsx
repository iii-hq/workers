import { type Host, type LiveAnnouncement, LiveRegion, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { useContainerNarrow, usePaneState, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useEffect, useMemo, useRef, useState } from 'react'
import { createEvalApi } from '../api'
import { parseOpenContext, type Setup } from '../model'
import { CycleDetail } from './CycleDetail'
import { Home } from './Home'
import { LoopSettings } from './LoopSettings'
import { NewCycle } from './NewCycle'

const NARROW_BELOW = 720
/** Cycle records, their budget ledgers and the loop config live in these state scopes; a change re-reads them. */
const CYCLE_TRIGGERS = [
  { type: 'state', config: { scope: 'eval_cycle' } },
  { type: 'state', config: { scope: 'eval_cycle_budget' } },
]
const CONFIG_TRIGGERS = [{ type: 'state', config: { scope: 'eval_cycle_config' } }]

/** What the pane shows: the cycles, one cycle, or the settings at a section. */
export type View = { kind: 'home' } | { kind: 'cycle'; cycleId: string } | { kind: 'settings'; section?: Setup }

export interface Nav {
  home(): void
  cycle(cycleId: string): void
  settings(section?: Setup): void
  create(): void
  announce(text: string): void
}

const HOME: View = { kind: 'home' }

function parseView(value: unknown): View {
  const view = value as Partial<{ kind: string; cycleId: unknown; section: Setup }> | null
  if (view?.kind === 'cycle' && typeof view.cycleId === 'string' && view.cycleId) {
    return { kind: 'cycle', cycleId: view.cycleId }
  }
  if (view?.kind === 'settings') return { kind: 'settings', section: view.section }
  return HOME
}

export function EvalPage({
  host,
  tabId = '',
  paneId,
  panelContext,
  onRequestClose,
  setDirty,
}: { host: Host } & Partial<PageRenderProps>) {
  const api = useMemo(() => createEvalApi(host), [host])
  const { ref, narrow } = useContainerNarrow({ below: NARROW_BELOW })
  const list = useWorkerLive({
    iii: host.iii,
    triggers: CYCLE_TRIGGERS,
    fetch: api.list,
    handlerId: 'iii::eval-ui::cycles',
  })
  const config = useWorkerLive({
    iii: host.iii,
    triggers: CONFIG_TRIGGERS,
    fetch: api.config,
    handlerId: 'iii::eval-ui::config',
  })
  // Persisted per pane: a reload reopens the same view.
  const [stored, setView] = usePaneState<View>(`eval:view:${paneId || tabId || 'page'}`, HOME)
  const view = parseView(stored)
  const [creating, setCreating] = useState(false)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const handled = useRef(-1)

  const nav = useMemo<Nav>(
    () => ({
      home: () => setView(HOME),
      cycle: (cycleId) => setView({ kind: 'cycle', cycleId }),
      settings: (section) => setView({ kind: 'settings', section }),
      create: () => setCreating(true),
      announce: (text) => setAnnouncement((previous) => ({ seq: (previous?.seq ?? 0) + 1, text, urgency: 'polite' })),
    }),
    [setView],
  )

  // `host.panels.open` from the palette or a command: a cycle, the new cycle dialog or the settings.
  useEffect(() => {
    if (!panelContext || panelContext.id === handled.current) return
    handled.current = panelContext.id
    const request = parseOpenContext(panelContext.context)
    if (request?.cycleId) nav.cycle(request.cycleId)
    if (request?.config) nav.settings()
    if (request?.start) {
      nav.home()
      nav.create()
    }
  }, [panelContext, nav])

  const current = config.data?.config?.config ?? null

  return (
    <PageShell className="eval-ui-shell">
      <div ref={ref} className="eval-ui-root" data-narrow={narrow ? 'true' : undefined}>
        <LiveRegion announcement={announcement} />
        {view.kind === 'cycle' ? (
          <CycleDetail
            key={view.cycleId}
            host={host}
            api={api}
            cycleId={view.cycleId}
            listed={list.data?.cycles.find((cycle) => cycle.cycle_id === view.cycleId)}
            current={current}
            narrow={narrow}
            nav={nav}
            onClose={onRequestClose}
            onChanged={list.refresh}
          />
        ) : view.kind === 'settings' ? (
          <LoopSettings
            api={api}
            state={config.data}
            error={config.error}
            cycles={list.data?.cycles ?? []}
            section={view.section}
            narrow={narrow}
            nav={nav}
            onClose={onRequestClose}
            onSaved={config.refresh}
            onDirtyChange={(dirty) => setDirty?.(dirty)}
          />
        ) : (
          <Home
            list={list}
            config={current}
            revision={config.data?.config?.revision}
            narrow={narrow}
            nav={nav}
            onClose={onRequestClose}
          />
        )}
        <NewCycle
          api={api}
          open={creating}
          onOpenChange={setCreating}
          cycles={list.data?.cycles ?? []}
          config={current}
          nav={nav}
          onStarted={list.refresh}
        />
      </div>
    </PageShell>
  )
}
