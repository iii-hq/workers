import {
  Button,
  EmptyState,
  type Host,
  List,
  ListItem,
  type LiveAnnouncement,
  LiveRegion,
  PageBody,
  PageMain,
  PageSidebar,
  type PanelSide,
  Skeleton,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Settings2 } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { EvalApi } from '../api'
import { cycleTitle, isActive, outcomeView, stageLabel, statusView, type Tone } from '../model'
import type { CycleRecord } from '../types'
import { CycleDetail } from './CycleDetail'
import { LoopSettings } from './LoopSettings'
import { StartCycle } from './StartCycle'

const NARROW_BELOW = 720
/** Cycle records and the loop config live in these state scopes; a change re-reads them. */
const CYCLE_TRIGGERS = [{ type: 'state', config: { scope: 'eval_cycle' } }]
const CONFIG_TRIGGERS = [{ type: 'state', config: { scope: 'eval_cycle_config' } }]
const DOT: Record<Tone, 'accent' | 'alert' | 'warn' | 'ok' | 'ink'> = {
  neutral: 'ink',
  accent: 'accent',
  ok: 'ok',
  warn: 'warn',
  alert: 'alert',
}

export interface OpenRequest {
  seq: number
  cycleId?: string
  start?: boolean
  config?: boolean
}

export function CycleView({
  host,
  api,
  paneKey,
  panelSide,
  openRequest,
  setDirty,
}: {
  host: Host
  api: EvalApi
  paneKey: string
  panelSide: PanelSide
  openRequest: OpenRequest | null
  setDirty?: (dirty: boolean) => void
}) {
  const { ref, narrow } = useContainerNarrow({ below: NARROW_BELOW })
  const cyclesLive = useWorkerLive({
    iii: host.iii,
    triggers: CYCLE_TRIGGERS,
    fetch: api.list,
    handlerId: 'iii::eval-ui::cycles',
  })
  const configLive = useWorkerLive({
    iii: host.iii,
    triggers: CONFIG_TRIGGERS,
    fetch: api.config,
    handlerId: 'iii::eval-ui::config',
  })
  const cycles = cyclesLive.data
  // Persisted per pane: a reload reopens the same cycle.
  const [stored, setSelected] = usePaneState<string | null>(`eval:cycle:${paneKey}`, null)
  const selectedId = typeof stored === 'string' && stored ? stored : null
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [drilled, setDrilled] = useState(false)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const inputRef = useRef<HTMLInputElement | null>(null)
  const mainRef = useRef<HTMLDivElement | null>(null)

  const announce = useCallback((text: string) => {
    setAnnouncement((previous) => ({ seq: (previous?.seq ?? 0) + 1, text, urgency: 'polite' }))
  }, [])

  const open = useCallback(
    (cycleId: string) => {
      setSelected(cycleId)
      setSettingsOpen(false)
      setDrilled(true)
    },
    [setSelected],
  )

  const openSettings = useCallback(() => {
    setSettingsOpen(true)
    setDrilled(true)
  }, [])

  useEffect(() => {
    if (!openRequest) return
    if (openRequest.cycleId) open(openRequest.cycleId)
    if (openRequest.config) openSettings()
    if (openRequest.start) {
      setDrilled(false)
      inputRef.current?.focus()
    }
  }, [openRequest, open, openSettings])

  useEffect(() => {
    if (narrow && drilled) mainRef.current?.focus({ preventScroll: true })
  }, [narrow, drilled])

  const onSettingsDirty = useCallback((dirty: boolean) => setDirty?.(dirty), [setDirty])
  const unconfigured = configLive.data !== null && configLive.data.config === null
  const selected = cycles?.find((cycle) => cycle.cycle_id === selectedId)
  const hasMain = settingsOpen || selectedId !== null
  const showMain = !narrow || (drilled && hasMain)
  const showList = !narrow || !showMain
  const back = narrow ? () => setDrilled(false) : undefined

  return (
    <div ref={ref} className="eval-ui-root" data-narrow={narrow ? 'true' : undefined}>
      <LiveRegion announcement={announcement} />
      <PageBody side={panelSide} className="eval-ui-body">
        {showList ? (
          <PageSidebar label="Cycles" side={panelSide} narrow={narrow} defaultWidth={320} className="eval-ui-sidebar">
            <div className="eval-ui-side">
              <div className="eval-ui-side-head">
                <span className="eval-ui-label">Loop</span>
                <Button
                  variant="ghost"
                  size={narrow ? 'lg' : 'sm'}
                  aria-pressed={settingsOpen}
                  onClick={() => (settingsOpen && !narrow ? setSettingsOpen(false) : openSettings())}
                >
                  <Settings2 size={16} aria-hidden /> Settings
                </Button>
              </div>
              {unconfigured ? (
                <StatusPanel
                  variant="warn"
                  headline="Loop not configured"
                  detail="Cycles map coverage but dispatch no E2E execution until settings are saved."
                />
              ) : null}
              <StartCycle
                api={api}
                inputRef={inputRef}
                onStarted={(cycleId) => {
                  announce('Cycle started')
                  open(cycleId)
                  cyclesLive.refresh()
                }}
              />
              <History
                cycles={cycles}
                loading={cyclesLive.loading}
                error={cyclesLive.error}
                selectedId={settingsOpen ? null : selectedId}
                onSelect={open}
                onRetry={cyclesLive.refresh}
              />
            </div>
          </PageSidebar>
        ) : null}
        {showMain ? (
          <PageMain className="eval-ui-main">
            <div ref={mainRef} className="eval-ui-main-scroll" tabIndex={-1}>
              {settingsOpen ? (
                <LoopSettings
                  api={api}
                  state={configLive.data}
                  error={configLive.error}
                  onSaved={() => {
                    announce('Loop settings saved')
                    configLive.refresh()
                  }}
                  onClose={back ?? (() => setSettingsOpen(false))}
                  onDirtyChange={onSettingsDirty}
                />
              ) : selectedId ? (
                <CycleDetail
                  key={selectedId}
                  host={host}
                  api={api}
                  cycleId={selectedId}
                  listed={selected}
                  onBack={back}
                  onChanged={(text) => {
                    announce(text)
                    cyclesLive.refresh()
                  }}
                />
              ) : (
                <div className="eval-ui-main-empty">
                  <EmptyState
                    title={cycles?.length ? 'Select a cycle' : 'No cycles yet'}
                    description={
                      cycles?.length
                        ? 'Pick one from the history to see its coverage, evidence and decision.'
                        : 'Start one from an objective, or evaluate a finished E2E execution.'
                    }
                  />
                </div>
              )}
            </div>
          </PageMain>
        ) : null}
      </PageBody>
    </div>
  )
}

function History({
  cycles,
  loading,
  error,
  selectedId,
  onSelect,
  onRetry,
}: {
  cycles: CycleRecord[] | null
  loading: boolean
  error: string | null
  selectedId: string | null
  onSelect: (cycleId: string) => void
  onRetry: () => void
}) {
  if (error && !cycles) {
    return (
      <StatusPanel
        variant="alert"
        headline="Couldn't read the cycles"
        detail={error}
        action={
          <Button variant="ghost" size="sm" onClick={onRetry}>
            Retry
          </Button>
        }
      />
    )
  }
  if (!cycles) {
    return (
      <div className="eval-ui-skeleton" role="status" aria-busy={loading || undefined} aria-label="Loading cycles">
        <Skeleton className="eval-ui-sk-row" />
        <Skeleton className="eval-ui-sk-row" />
        <Skeleton className="eval-ui-sk-row" />
      </div>
    )
  }
  if (cycles.length === 0) return <EmptyState compact title="No cycles yet" description="Started cycles appear here." />
  return (
    <nav aria-label="Cycle history">
      <List className="eval-ui-history">
        {cycles.map((cycle) => {
          const view = outcomeView(cycle) ?? statusView(cycle.status)
          const source = 'objective' in cycle.input ? 'objective' : 'execution'
          return (
            <ListItem
              key={cycle.cycle_id}
              selected={cycle.cycle_id === selectedId}
              aria-current={cycle.cycle_id === selectedId ? 'true' : undefined}
              onClick={() => onSelect(cycle.cycle_id)}
              leading={<StatusDot tone={DOT[view.tone]} pulse={isActive(cycle.status)} />}
              label={<span className="eval-ui-row-title">{cycleTitle(cycle)}</span>}
              description={`${view.label} · ${stageLabel(cycle.stage)} · ${source}`}
              trailing={<span className="eval-ui-row-time">{formatRelative(cycle.updated_at)}</span>}
            />
          )
        })}
      </List>
    </nav>
  )
}
