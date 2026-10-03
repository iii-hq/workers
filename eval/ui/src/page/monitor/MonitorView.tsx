import {
  EmptyState,
  type Host,
  type LiveAnnouncement,
  LiveRegion,
  PageBody,
  type PageCommandsApi,
  PageMain,
  type PanelSide,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import type { EvalApi } from '../../api'
import { useAnalysisCompleted } from '../../events'
import { type Filter, isActive } from '../../model'
import type { AnalysisRecord, CompletedEvent, MonitorConfig, MonitorState } from '../../types'
import { AnalysisDetail } from './detail/Detail'
import { canOpenSession, openSession } from './open-session'
import { MonitorSettings } from './Settings'
import { Sidebar } from './Sidebar'
import { isDeletedAnalysis, LIST_LIMIT, NARROW_BELOW, needsPolling, type OpenRequest, POLL_MS } from './shell-state'

export interface MonitorViewProps {
  host: Host
  api: EvalApi
  /** `paneId ?? tabId`: two panes of this page keep separate selections. */
  paneKey: string
  panelSide: PanelSide
  /** A palette row or command asking for an analysis, or for the Analyze field. */
  openRequest: OpenRequest | null
  onOpenHandled: () => void
  /** `PageRenderProps.commands`: the page's rows in the command palette. */
  commands?: PageCommandsApi
  /** `PageRenderProps.setDirty`: the host asks before closing a pane with unsaved settings. */
  setDirty?: (dirty: boolean) => void
}

type Level = 'both' | 'list' | 'main'

export function MonitorView({
  host,
  api,
  paneKey,
  panelSide,
  openRequest,
  onOpenHandled,
  commands,
  setDirty,
}: MonitorViewProps) {
  const { ref: measure, narrow } = useContainerNarrow({ below: NARROW_BELOW })
  const rootRef = useRef<HTMLDivElement | null>(null)
  const ref = useCallback(
    (element: HTMLDivElement | null) => {
      rootRef.current = element
      measure(element)
    },
    [measure],
  )
  const [monitor, setMonitor] = useState<MonitorState | null>(null)
  const [monitorError, setMonitorError] = useState<string | null>(null)
  const [records, setRecords] = useState<AnalysisRecord[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [filter, setFilter] = useState<Filter>('all')
  const [storedSelection, setSelectedId] = usePaneState<string | null>(`eval:selection:${paneKey}`, null)
  const selectedId = typeof storedSelection === 'string' && storedSelection ? storedSelection : null
  const [settingsOpen, setSettingsOpen] = useState(false)
  /** Narrow only: the detail covers the list after a row is opened. */
  const [drilled, setDrilled] = useState(false)
  const [refreshKey, setRefreshKey] = useState(0)
  const [now, setNow] = useState(() => Date.now())
  const [togglePending, setTogglePending] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const { confirm, dialog } = useConfirm()

  // Responses carry the counter they were requested with; an older one never
  // overwrites a newer one.
  const listSeq = useRef(0)
  const monitorSeq = useRef(0)
  const recordsRef = useRef<AnalysisRecord[] | null>(null)
  // A list read that is still out: the poll waits for it instead of discarding it.
  const listInFlight = useRef(false)
  // After a delete, focus returns to the list once it is showing.
  const focusList = useRef(false)
  const selectedRef = useRef(selectedId)
  selectedRef.current = selectedId
  const inputRef = useRef<HTMLInputElement>(null)
  const mainRef = useRef<HTMLDivElement>(null)
  const listScroll = useRef(0)
  const restoreFocus = useRef(false)
  const handledRequest = useRef(-1)
  const settingsDirty = useRef(false)
  const unlisted = useRef(new Set<string>())

  const announce = useCallback((text: string) => {
    setAnnouncement((previous) => ({ seq: (previous?.seq ?? 0) + 1, text, urgency: 'polite' }))
  }, [])

  const loadList = useCallback(async () => {
    const seq = ++listSeq.current
    listInFlight.current = true
    try {
      const next = await api.list()
      if (seq !== listSeq.current) return
      const id = selectedRef.current
      const before = recordsRef.current?.find((record) => record.evaluation_id === id)
      const after = next.find((record) => record.evaluation_id === id)
      // The open analysis moved on (a stage, a result): its detail reloads.
      if (before && after && (before.updated_at !== after.updated_at || before.status !== after.status)) {
        setRefreshKey((key) => key + 1)
      }
      recordsRef.current = next
      setRecords(next)
      setListError(null)
      setNow(Date.now())
    } catch (error) {
      if (seq === listSeq.current) setListError(errorMessage(error))
    } finally {
      if (seq === listSeq.current) listInFlight.current = false
    }
  }, [api])

  const loadMonitor = useCallback(async () => {
    const seq = ++monitorSeq.current
    try {
      const next = await api.monitor(false)
      if (seq !== monitorSeq.current) return
      setMonitor(next)
      setMonitorError(null)
    } catch (error) {
      if (seq === monitorSeq.current) setMonitorError(errorMessage(error))
    }
  }, [api])

  const reload = useCallback(() => {
    void loadList()
    void loadMonitor()
  }, [loadList, loadMonitor])

  useEffect(reload, [reload])

  const onCompleted = useCallback(
    (event: CompletedEvent) => {
      reload()
      if (event.evaluation_id === selectedRef.current) setRefreshKey((key) => key + 1)
    },
    [reload],
  )
  useAnalysisCompleted(host, onCompleted)

  // Poll while an analysis is unfinished or the observer has not bound yet.
  const polling = needsPolling(records, monitor)
  useEffect(() => {
    if (!polling) return
    const timer = window.setInterval(() => {
      // A read slower than the interval lands instead of being replaced by the next one.
      if (!document.hidden && !listInFlight.current) reload()
    }, POLL_MS)
    return () => window.clearInterval(timer)
  }, [polling, reload])

  // Coming back to the browser tab: refresh what went stale.
  useEffect(() => {
    const onVisible = () => {
      if (!document.hidden) reload()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => document.removeEventListener('visibilitychange', onVisible)
  }, [reload])

  // Keeps "today" and the capacity notice honest between refreshes.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000)
    return () => window.clearInterval(timer)
  }, [])

  // A selection whose analysis is gone (deleted elsewhere, or a saved one that
  // outlived its retention): the list is complete below its limit, so absence
  // there means gone. An analysis selected a moment ago may not be listed
  // yet, so it only counts once a list has shown it.
  useEffect(() => {
    if (!records || listError) return
    for (const record of records) unlisted.current.delete(record.evaluation_id)
    if (!selectedId || unlisted.current.has(selectedId) || records.length >= LIST_LIMIT) return
    if (!records.some((record) => record.evaluation_id === selectedId)) setSelectedId(null)
  }, [records, listError, selectedId, setSelectedId])

  const select = useCallback(
    (evaluationId: string) => {
      unlisted.current.add(evaluationId)
      setSelectedId(evaluationId)
      setSettingsOpen(false)
      setDrilled(true)
    },
    [setSelectedId],
  )

  const onSettingsDirty = useCallback(
    (dirty: boolean) => {
      settingsDirty.current = dirty
      setDirty?.(dirty)
    },
    [setDirty],
  )

  /** Unsaved settings are never dropped silently by navigating elsewhere. */
  const leaveSettings = useCallback(async () => {
    if (!settingsDirty.current) return true
    return confirm({
      title: 'Discard unsaved changes?',
      description: "Your edits to the monitor settings haven't been saved.",
      confirmLabel: 'Discard',
      cancelLabel: 'Keep editing',
      tone: 'danger',
    })
  }, [confirm])

  const openAnalysis = useCallback(
    async (evaluationId: string) => {
      if (await leaveSettings()) select(evaluationId)
    },
    [leaveSettings, select],
  )

  const toggleSettings = useCallback(async () => {
    if (settingsOpen && !(await leaveSettings())) return
    setSettingsOpen(!settingsOpen)
  }, [leaveSettings, settingsOpen])

  const backToList = useCallback(() => {
    restoreFocus.current = true
    setDrilled(false)
  }, [])

  useEffect(() => {
    if (!openRequest || handledRequest.current === openRequest.seq) return
    handledRequest.current = openRequest.seq
    if (openRequest.type === 'analysis') {
      void openAnalysis(openRequest.evaluationId)
    } else {
      void leaveSettings().then((leave) => {
        if (!leave) return
        setSettingsOpen(false)
        setDrilled(false)
        window.requestAnimationFrame(() => inputRef.current?.focus())
      })
    }
    onOpenHandled()
  }, [openRequest, openAnalysis, leaveSettings, onOpenHandled])

  const analyze = useCallback(
    async (sessionId: string) => {
      const started = await api.analyze(sessionId).catch((error: unknown) => {
        // The turn of a deleted analysis stays indexed and only `reanalyze` admits it
        // again. The id was typed on purpose, so that is what the form means.
        if (isDeletedAnalysis(errorMessage(error))) return api.analyze(sessionId, true)
        throw error
      })
      await loadList()
      // Open settings with edits stay put; the analysis shows up in the list.
      const shown = !settingsDirty.current
      if (shown) select(started.evaluation_id)
      announce(started.reused && shown ? 'Already analyzed — showing the existing analysis' : `Analyzing ${sessionId}`)
      return { reused: started.reused && shown }
    },
    [api, announce, loadList, select],
  )

  const toggleObservation = useCallback(async () => {
    const config = monitor?.config
    if (!config || togglePending) return
    setTogglePending(true)
    setActionError(null)
    try {
      const next = await api.toggle(config)
      setMonitor((previous) => (previous ? { ...previous, config: next } : previous))
      announce(next.enabled ? 'Observation resumed' : 'Observation paused')
      void loadMonitor()
    } catch (error) {
      setActionError(errorMessage(error))
    } finally {
      setTogglePending(false)
    }
  }, [announce, api, loadMonitor, monitor?.config, togglePending])

  const saved = useCallback(
    (config: MonitorConfig) => {
      setMonitor((previous) => (previous ? { ...previous, config } : previous))
      void loadMonitor()
    },
    [loadMonitor],
  )

  const deleted = useCallback(() => {
    // The row goes now, so focus has a stable first row to land on.
    const gone = selectedRef.current
    if (gone && recordsRef.current) {
      recordsRef.current = recordsRef.current.filter((record) => record.evaluation_id !== gone)
      setRecords(recordsRef.current)
    }
    focusList.current = true
    setSelectedId(null)
    setDrilled(false)
    reload()
  }, [reload, setSelectedId])

  const firstRun = monitor !== null && monitor.config === null
  const kind = firstRun || settingsOpen ? 'settings' : selectedId ? 'detail' : 'empty'
  const level: Level = !narrow ? 'both' : kind === 'settings' || (kind === 'detail' && drilled) ? 'main' : 'list'

  // The list reads `restoreFocus` in the render that follows the request;
  // afterwards it is spent.
  useEffect(() => {
    restoreFocus.current = false
  })

  // The Delete button left with the detail: focus goes to the first row, or to
  // the Analyze field when the list is empty.
  useEffect(() => {
    if (!focusList.current || level === 'main') return
    focusList.current = false
    const row = rootRef.current?.querySelector<HTMLElement>('.eval-ui-history .eval-ui-row')
    ;(row ?? inputRef.current)?.focus({ preventScroll: true })
  })

  // Narrow: the detail replaced the list, so focus follows it.
  const previousLevel = useRef(level)
  useEffect(() => {
    if (narrow && previousLevel.current === 'list' && level === 'main') mainRef.current?.focus({ preventScroll: true })
    previousLevel.current = level
  }, [level, narrow])

  const activeCount = records?.filter((record) => isActive(record.status)).length ?? 0

  // The observed session of the analysis on screen, for the palette command.
  const canOpen = canOpenSession(host)
  const observed = useRef<string | undefined>(undefined)
  observed.current =
    kind === 'detail' ? records?.find((record) => record.evaluation_id === selectedId)?.session_id : undefined

  const openObserved = useCallback(
    (sessionId: string) => {
      if (!openSession(host, sessionId)) announce("This console can't open that session.")
    },
    [announce, host],
  )

  useEffect(() => {
    if (!commands || !canOpen) return
    return commands.register([
      {
        id: 'open-observed-session',
        title: 'Open the observed session',
        detail: 'Show the session of the selected analysis beside this page',
        keywords: ['session', 'conversation', 'chat', 'original', 'observed'],
        enabled: () => observed.current !== undefined,
        run: () => {
          if (observed.current) openObserved(observed.current)
        },
      },
    ])
  }, [commands, canOpen, openObserved])

  let main: ReactNode = null
  if (kind === 'settings') {
    main = (
      <MonitorSettings
        api={api}
        state={monitor}
        limits={monitor?.limits}
        runningCount={activeCount}
        onSaved={saved}
        narrow={narrow}
        onClose={firstRun ? undefined : () => setSettingsOpen(false)}
        onDirtyChange={onSettingsDirty}
      />
    )
  } else if (kind === 'detail' && selectedId) {
    main = (
      <AnalysisDetail
        key={selectedId}
        host={host}
        api={api}
        evaluationId={selectedId}
        narrow={narrow}
        limits={monitor?.limits}
        refreshKey={refreshKey}
        onBack={narrow ? backToList : undefined}
        onSelect={select}
        onChanged={reload}
        onDeleted={deleted}
      />
    )
  } else if (records) {
    main = (
      <div className="eval-ui-main-empty">
        {records.length > 0 ? (
          <EmptyState
            title="Select an analysis"
            description="Pick one from the list to see its signals, triage and suggestions."
          />
        ) : (
          <EmptyState title="No analyses yet" description="Finished sessions show up here once the monitor is on." />
        )}
      </div>
    )
  }

  return (
    <div ref={ref} className="eval-ui-monitor" data-narrow={narrow ? 'true' : undefined}>
      {dialog}
      <LiveRegion announcement={announcement} />
      <PageBody side={panelSide} className="eval-ui-body">
        {level !== 'main' ? (
          <Sidebar
            panelSide={panelSide}
            narrow={narrow}
            state={monitor}
            stateError={monitorError}
            records={records}
            listError={listError}
            now={now}
            filter={filter}
            onFilter={setFilter}
            selectedId={selectedId}
            settingsOpen={settingsOpen}
            togglePending={togglePending}
            actionError={actionError}
            onToggleObservation={() => void toggleObservation()}
            onToggleSettings={() => void toggleSettings()}
            onSelect={(evaluationId) => void openAnalysis(evaluationId)}
            onAnalyze={analyze}
            onOpenSession={canOpen ? openObserved : undefined}
            onRetry={reload}
            inputRef={inputRef}
            scrollMemo={listScroll}
            restoreFocus={restoreFocus.current}
          />
        ) : null}
        {level !== 'list' ? (
          <PageMain className="eval-ui-main" data-kind={kind}>
            <div
              // Another analysis, or settings in place of one, starts at the top.
              key={kind === 'detail' ? selectedId : kind}
              ref={mainRef}
              className="eval-ui-main-scroll"
              tabIndex={-1}
              aria-busy={records === null && !listError ? true : undefined}
            >
              {main}
            </div>
          </PageMain>
        ) : null}
      </PageBody>
    </div>
  )
}
