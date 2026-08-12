/**
 * The canvas page (#/ext/canvas): standard page chrome (shell explorer
 * composition) over a sidebar of stored canvases and a per-format main
 * surface — the mermaid editor/preview split, or the freeform whiteboard.
 *
 * The sidebar lists `canvas::list` (name, family badge, updated-at),
 * creates via `canvas::create` (starter flowchart), deletes with confirm,
 * and selection loads fresh source via `canvas::get`. Unsaved drafts live
 * in a page-owned cache keyed by canvas id (shell EditorPane pattern), so
 * switching canvases keeps edits; dirty ids get a dot in the list.
 */

import {
  Badge,
  Button,
  EmptyState,
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
  PageSidebar,
  StatusPanel,
  type Host,
  type PageRenderProps,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { FreeformPane, type FreeformPaneHandle } from '../freeform'
import { useCanvasStateEvents } from '../lib/live'
import type { CanvasRecord } from '../lib/types'
import {
  createCanvas,
  deleteCanvas,
  getCanvas,
  listCanvases,
  updateCanvas,
} from './data'
import {
  NEW_CANVAS_NAME,
  STARTER_FLOWCHART,
  errorMessage,
  familyBadgeLabel,
  relativeTime,
} from './helpers'
import { AlertCircle, Plus, Shapes, Trash2 } from './icons'
import { type DraftCache, MermaidPane } from './MermaidPane'

type ListState =
  | { phase: 'loading' }
  | { phase: 'error'; message: string }
  | { phase: 'ready' }

const SIDEBAR_WIDTH_KEY = 'canvas-ui:sidebar-width'
const SIDEBAR_DEFAULT_WIDTH = 248
const SIDEBAR_MIN_WIDTH = 190
const SIDEBAR_MAX_WIDTH = 440

function clampSidebarWidth(w: number): number {
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, Math.round(w)))
}

function initialSidebarWidth(): number {
  const stored = Number(window.localStorage.getItem(SIDEBAR_WIDTH_KEY))
  return Number.isFinite(stored) && stored > 0
    ? clampSidebarWidth(stored)
    : SIDEBAR_DEFAULT_WIDTH
}

/** Most recently touched first; name breaks timestamp ties stably. */
function byRecency(a: CanvasRecord, b: CanvasRecord): number {
  return b.updated_at - a.updated_at || a.name.localeCompare(b.name)
}

const ShapesGlyph = (p: { className?: string }) => <Shapes size={28} {...p} />

export function CanvasPage({
  host,
  panelSide,
  onRequestClose,
}: { host: Host } & PageRenderProps) {
  const [canvases, setCanvases] = useState<CanvasRecord[]>([])
  const [listState, setListState] = useState<ListState>({ phase: 'loading' })
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [record, setRecord] = useState<CanvasRecord | null>(null)
  const [recordError, setRecordError] = useState<string | null>(null)
  const [dirtyIds, setDirtyIds] = useState<ReadonlySet<string>>(new Set())
  const [creating, setCreating] = useState(false)
  const [sideError, setSideError] = useState<string | null>(null)
  const cacheRef = useRef<DraftCache>(new Map())
  const loadSeqRef = useRef(0)
  // The open record, readable from stable callbacks (the freeform surface
  // binds onSave once; a state closure would go stale across saves).
  const recordRef = useRef<CanvasRecord | null>(null)
  useEffect(() => {
    recordRef.current = record
  }, [record])
  // The freeform surface's imperative handle: flush() before switching away
  // so a settling whiteboard edit is persisted, never silently dropped.
  const freeformHandleRef = useRef<FreeformPaneHandle | null>(null)
  // Bumped when the OPEN record changes under us (an agent edit streaming
  // in) — the pane keys include it, so a remount rehydrates from the
  // refreshed cache. Own saves must NOT bump it: a remount mid-typing
  // would reset the editor's cursor for no reason.
  const [externalBump, setExternalBump] = useState(0)
  const selectedIdRef = useRef<string | null>(null)
  useEffect(() => {
    selectedIdRef.current = selectedId
  }, [selectedId])

  // ── sidebar width (drag handle on the boundary toward the main pane) ──
  const [sideWidth, setSideWidth] = useState(initialSidebarWidth)
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null)
  const onHandlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      dragRef.current = { startX: e.clientX, startWidth: sideWidth }
      try {
        e.currentTarget.setPointerCapture(e.pointerId)
      } catch {
        // capture is best-effort; the drag still tracks over the handle
      }
    },
    [sideWidth],
  )
  const onHandlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current
      if (!drag) return
      const delta = e.clientX - drag.startX
      // A right-hugging sidebar widens as the handle moves left.
      setSideWidth(
        clampSidebarWidth(
          panelSide === 'right'
            ? drag.startWidth - delta
            : drag.startWidth + delta,
        ),
      )
    },
    [panelSide],
  )
  const onHandlePointerUp = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (dragRef.current) {
        window.localStorage.setItem(
          SIDEBAR_WIDTH_KEY,
          String(clampSidebarWidth(sideWidth)),
        )
      }
      dragRef.current = null
      try {
        e.currentTarget.releasePointerCapture(e.pointerId)
      } catch {
        // never captured
      }
    },
    [sideWidth],
  )

  const refreshList = useCallback(() => {
    listCanvases(host)
      .then((out) => {
        setCanvases([...out.canvases].sort(byRecency))
        setListState({ phase: 'ready' })
      })
      .catch((err: unknown) => {
        setListState({ phase: 'error', message: errorMessage(err) })
      })
  }, [host])

  useEffect(() => {
    refreshList()
  }, [refreshList])

  /**
   * An agent edit streamed in for the OPEN record: re-fetch, refresh the
   * draft cache (a clean buffer follows the worker, a dirty one keeps the
   * user's edits), and remount the pane via the bump key. Own saves are
   * already applied by mergeSaved — the identity check makes their echo
   * event a no-op, so saving never resets the editor.
   */
  const refreshOpenRecord = useCallback(() => {
    const id = selectedIdRef.current
    if (!id) return
    getCanvas(host, id)
      .then((rec) => {
        if (selectedIdRef.current !== rec.id) return
        const cache = cacheRef.current
        const entry = cache.get(rec.id)
        const cur = recordRef.current
        // id + updated_at matching = this is our own save echoing back —
        // freeform saves have no cache entry, so the entry must not gate
        // the check (a gated echo would remount the whiteboard mid-edit).
        const alreadyCurrent =
          cur !== null && cur.id === rec.id && cur.updated_at === rec.updated_at
        if (alreadyCurrent) {
          if (entry && entry.draft === entry.savedSource) {
            entry.savedSource = rec.source
            entry.draft = rec.source
          }
          return
        }
        if (!entry) {
          cache.set(rec.id, { savedSource: rec.source, draft: rec.source })
        } else {
          const wasClean = entry.draft === entry.savedSource
          entry.savedSource = rec.source
          if (wasClean) entry.draft = rec.source
        }
        setRecord(rec)
        setExternalBump((n) => n + 1)
      })
      .catch(() => {
        // A racing delete lands as its own state event; nothing to do here.
      })
  }, [host])

  // Live updates: canvas records live in state scope "canvas", so every
  // agent-side create/update/delete emits a state event this tab streams
  // (one list on mount, pushes after — never polling). Events burst (a
  // create writes record + index), so a short window coalesces them.
  const liveTimerRef = useRef<number | null>(null)
  const livePendingRef = useRef({
    list: false,
    record: false,
    gone: false,
    created: null as string | null,
  })
  useCanvasStateEvents(host, (event) => {
    const pending = livePendingRef.current
    pending.list = true
    const openId = selectedIdRef.current
    if (openId && event.key === `record/${openId}`) {
      if (event.event_type === 'state:deleted') pending.gone = true
      else pending.record = true
    }
    // An agent explaining something with a fresh diagram should surface it
    // without a click — but never by stealing focus from an OPEN canvas.
    if (
      event.event_type === 'state:created' &&
      event.key.startsWith('record/')
    ) {
      pending.created = event.key.slice('record/'.length)
    }
    if (liveTimerRef.current !== null) return
    liveTimerRef.current = window.setTimeout(() => {
      liveTimerRef.current = null
      const todo = { ...pending }
      pending.list = false
      pending.record = false
      pending.gone = false
      pending.created = null
      if (todo.gone) {
        ++loadSeqRef.current
        setSelectedId(null)
        setRecord(null)
        setRecordError(null)
        recordRef.current = null
        setSideError('the open canvas was deleted elsewhere')
      } else if (todo.record) {
        refreshOpenRecord()
      }
      if (todo.list) refreshList()
      if (todo.created && selectedIdRef.current === null) {
        select(todo.created)
      }
    }, 200)
  })
  useEffect(
    () => () => {
      if (liveTimerRef.current !== null) {
        window.clearTimeout(liveTimerRef.current)
      }
    },
    [],
  )

  const select = useCallback(
    (id: string) => {
      freeformHandleRef.current?.flush()
      const seq = ++loadSeqRef.current
      setSelectedId(id)
      setRecord(null)
      setRecordError(null)
      getCanvas(host, id)
        .then((rec) => {
          if (loadSeqRef.current !== seq) return
          // Merge the fresh source under any unsaved draft: a clean entry
          // follows the worker, a dirty one keeps the user's buffer.
          const cache = cacheRef.current
          const entry = cache.get(rec.id)
          if (!entry) {
            cache.set(rec.id, { savedSource: rec.source, draft: rec.source })
          } else {
            const wasClean = entry.draft === entry.savedSource
            entry.savedSource = rec.source
            if (wasClean) entry.draft = rec.source
          }
          setRecord(rec)
        })
        .catch((err: unknown) => {
          if (loadSeqRef.current === seq) setRecordError(errorMessage(err))
        })
    },
    [host],
  )

  const onDirtyChange = useCallback((id: string, dirty: boolean) => {
    setDirtyIds((prev) => {
      if (prev.has(id) === dirty) return prev
      const next = new Set(prev)
      if (dirty) next.add(id)
      else next.delete(id)
      return next
    })
  }, [])

  /** A save came back: refresh the open record and its list row. */
  const mergeSaved = useCallback((updated: CanvasRecord) => {
    setRecord((cur) => (cur && cur.id === updated.id ? updated : cur))
    setCanvases((list) =>
      list.map((c) => (c.id === updated.id ? updated : c)).sort(byRecency),
    )
  }, [])

  const create = useCallback(() => {
    if (creating) return
    freeformHandleRef.current?.flush()
    setCreating(true)
    setSideError(null)
    createCanvas(host, {
      name: NEW_CANVAS_NAME,
      format: 'mermaid',
      source: STARTER_FLOWCHART,
    })
      .then((rec) => {
        cacheRef.current.set(rec.id, {
          savedSource: rec.source,
          draft: rec.source,
        })
        setCanvases((list) => [rec, ...list])
        setListState({ phase: 'ready' })
        ++loadSeqRef.current
        setSelectedId(rec.id)
        setRecord(rec)
        setRecordError(null)
      })
      .catch((err: unknown) => setSideError(errorMessage(err)))
      .finally(() => setCreating(false))
  }, [host, creating])

  const remove = useCallback(
    (rec: CanvasRecord) => {
      const question = dirtyIds.has(rec.id)
        ? `delete "${rec.name}"? it has unsaved changes.`
        : `delete "${rec.name}"?`
      if (!window.confirm(question)) return
      setSideError(null)
      deleteCanvas(host, rec.id)
        .then(() => {
          cacheRef.current.delete(rec.id)
          setDirtyIds((prev) => {
            if (!prev.has(rec.id)) return prev
            const next = new Set(prev)
            next.delete(rec.id)
            return next
          })
          setCanvases((list) => list.filter((c) => c.id !== rec.id))
          if (selectedId === rec.id) {
            ++loadSeqRef.current
            setSelectedId(null)
            setRecord(null)
            setRecordError(null)
            // Immediately, not via the effect: a whiteboard edit settling
            // after the delete must not resurrect the record.
            recordRef.current = null
          }
        })
        .catch((err: unknown) => setSideError(errorMessage(err)))
    },
    [host, dirtyIds, selectedId],
  )

  /** The freeform surface's save path (`canvas::update` on the open record). */
  const saveFreeform = useCallback(
    (source: string) => {
      const rec = recordRef.current
      if (!rec || rec.format !== 'freeform') return
      updateCanvas(host, rec.id, source)
        .then(mergeSaved)
        .catch((err: unknown) => setSideError(errorMessage(err)))
    },
    [host, mergeSaved],
  )

  const nowSecs = Math.floor(Date.now() / 1000)

  return (
    <PageShell className="canvas-ui">
      <PageHeader
        icon={<Shapes />}
        title="canvas"
        description="diagrams stored as editable source"
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageSidebar width={sideWidth} className="cv-sidebar">
          <div
            className={`cv-resize-handle ${panelSide === 'right' ? 'left' : 'right'}`}
            onPointerDown={onHandlePointerDown}
            onPointerMove={onHandlePointerMove}
            onPointerUp={onHandlePointerUp}
            role="separator"
            aria-orientation="vertical"
            aria-label="resize sidebar"
            title="drag to resize"
          />
          <div className="cv-side-head">
            <span className="cv-side-count">
              {listState.phase === 'ready'
                ? `${canvases.length} canvas${canvases.length === 1 ? '' : 'es'}`
                : 'canvases'}
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={create}
              disabled={creating}
              title="new canvas (starter flowchart)"
            >
              <Plus size={13} aria-hidden />
              new
            </Button>
          </div>

          {sideError ? <div className="cv-note warn">{sideError}</div> : null}

          {listState.phase === 'loading' ? (
            <div className="cv-note">loading canvases…</div>
          ) : listState.phase === 'error' ? (
            <div className="cv-note warn">
              canvas::list failed: {listState.message}
            </div>
          ) : canvases.length === 0 ? (
            <div className="cv-note">no canvases yet — create one</div>
          ) : (
            <ul className="cv-list">
              {canvases.map((c) => (
                <li
                  key={c.id}
                  className={`cv-item${c.id === selectedId ? ' selected' : ''}`}
                >
                  <button
                    type="button"
                    className="cv-item-open"
                    onClick={() => select(c.id)}
                    title={`${c.name} · ${c.id}`}
                  >
                    <span className="cv-item-name">
                      <span className="cv-item-label">{c.name}</span>
                      {dirtyIds.has(c.id) ? (
                        <span className="cv-dirty" title="unsaved changes" />
                      ) : null}
                    </span>
                    <span className="cv-item-meta">
                      <Badge
                        variant={c.format === 'freeform' ? 'accent' : 'default'}
                      >
                        {familyBadgeLabel(c.format, c.family)}
                      </Badge>
                      <span className="cv-item-time">
                        {relativeTime(c.updated_at, nowSecs)}
                      </span>
                    </span>
                  </button>
                  <button
                    type="button"
                    className="cv-item-delete"
                    aria-label={`delete ${c.name}`}
                    title="delete"
                    onClick={() => remove(c)}
                  >
                    <Trash2 size={12} aria-hidden />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </PageSidebar>

        <PageMain className="cv-main">
          {selectedId === null ? (
            listState.phase === 'ready' && canvases.length === 0 ? (
              <EmptyState
                icon={ShapesGlyph}
                title="no canvases yet"
                description="create a canvas to start sketching — new canvases open with a starter flowchart."
                action={{ label: 'new canvas', onClick: create }}
              />
            ) : (
              <EmptyState
                icon={ShapesGlyph}
                title="canvas"
                description="select a canvas from the sidebar, or create a new one."
              />
            )
          ) : recordError ? (
            <StatusPanel
              variant="alert"
              icon={<AlertCircle size={16} />}
              headline="could not load the canvas"
              detail={recordError}
            />
          ) : record === null ? (
            <div className="cv-note pad">loading canvas…</div>
          ) : record.format === 'mermaid' ? (
            <MermaidPane
              key={`${record.id}:${externalBump}`}
              host={host}
              record={record}
              cache={cacheRef.current}
              onSaved={mergeSaved}
              onDirtyChange={onDirtyChange}
            />
          ) : (
            <FreeformPane
              // No bump: external updates apply IN PLACE via updateScene
              // (the pane's live-apply effect) — remounting would flash
              // the whiteboard on every streamed agent stroke.
              key={record.id}
              host={host}
              record={record}
              onSave={saveFreeform}
              handleRef={(handle) => {
                freeformHandleRef.current = handle
              }}
              onDirtyChange={(dirty) => onDirtyChange(record.id, dirty)}
            />
          )}
        </PageMain>
      </PageBody>
    </PageShell>
  )
}
