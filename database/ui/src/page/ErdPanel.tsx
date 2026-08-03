/**
 * Schema diagram.
 *
 * The worker does the layout — `database::schemaDiagram` returns positioned
 * nodes and already-routed edge polylines, so this panel only draws and pans.
 * That keeps the expensive part off the console's single React tree, and it
 * means an agent can read the same shape without rendering anything.
 *
 * Rendering follows the house pattern used by the worktree and memory pages:
 * absolutely-positioned HTML nodes over one SVG for the edges. Pan and zoom
 * live in a ref and write `transform` straight onto the canvas during
 * pointermove — putting them in state would re-render every node on every
 * mouse move.
 */

import { Badge, Button, EmptyState, type Host, Input, Select, StatusPanel } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { schemaDiagram } from '../lib/rpc'
import { AlertCircle, KeyRound, Link2, RefreshCw, Table2 } from './icons'
import { useDatabaseRead } from './useDatabaseRead'

const MIN_ZOOM = 0.15
const MAX_ZOOM = 2.5
/** Below this, column rows are unreadable anyway — drop them and keep 200
 *  headers instead of 2400 rows in the DOM. */
const COMPACT_BELOW = 0.5
const PAD = 60
/** Breathing room between a component's boundary and its outermost node. */
const GROUP_PAD = 22

const TableIcon = (p: { className?: string }) => <Table2 size={28} {...p} />

type Offset = { dx: number; dy: number } | undefined
type Pt = { x: number; y: number }

/**
 * Shift a routed edge to follow nodes that have been dragged.
 *
 * The worker emits a three-segment elbow: source anchor, two corner points in
 * the gutter, target anchor. Moving the ends alone would leave the corners
 * behind and bend the line through them, so each point is offset by a blend of
 * the two ends weighted by how far along the polyline it sits. With neither
 * end moved this is the identity.
 */
function shiftEdge(points: Pt[], from: Offset, to: Offset): Pt[] {
  if (!from && !to) return points
  const fx = from?.dx ?? 0
  const fy = from?.dy ?? 0
  const tx = to?.dx ?? 0
  const ty = to?.dy ?? 0
  const last = points.length - 1
  if (last <= 0) return points.map((p) => ({ x: p.x + fx, y: p.y + fy }))
  return points.map((p, i) => {
    const t = i / last
    return { x: p.x + fx * (1 - t) + tx * t, y: p.y + fy * (1 - t) + ty * t }
  })
}

interface View {
  x: number
  y: number
  z: number
}

export function ErdPanel({
  host,
  db,
  focusTable,
}: {
  host: Host
  db: string
  /** Open focused on this table rather than on the whole schema. */
  focusTable?: string | null
}) {
  // Focus turns the diagram from a wall into something explorable: one table
  // and what it touches, expanding a hop at a time.
  const [focus, setFocus] = useState<string | null>(focusTable ?? null)
  const [depth, setDepth] = useState(1)

  const fetcher = useCallback(() => schemaDiagram(host, db, { focus, depth }), [host, db, focus, depth])
  const read = useDatabaseRead(true, fetcher)
  const diagram = read.data

  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  // Manual node positions, as offsets over what the worker laid out. Kept in
  // the browser: this is where you dragged a box just now, not a preference
  // worth persisting for everyone.
  const [offsets, setOffsets] = useState<Record<string, { dx: number; dy: number }>>({})
  const nodeDragRef = useRef<{ table: string; px: number; py: number } | null>(null)
  // Mirrored: `view` drives React, `viewRef` drives the raw transform during a
  // drag so a 200-node diagram does not re-render per frame.
  const [view, setView] = useState<View>({ x: 0, y: 0, z: 1 })
  const viewRef = useRef<View>(view)
  const canvasRef = useRef<HTMLDivElement | null>(null)
  const frameRef = useRef<HTMLDivElement | null>(null)
  const dragRef = useRef<{ px: number; py: number } | null>(null)
  const rafRef = useRef<number | null>(null)

  const apply = useCallback(() => {
    rafRef.current = null
    const el = canvasRef.current
    if (!el) return
    const v = viewRef.current
    el.style.transform = `translate(${v.x}px, ${v.y}px) scale(${v.z})`
    el.classList.toggle('compact', v.z < COMPACT_BELOW)
  }, [])

  const schedule = useCallback(() => {
    if (rafRef.current == null) rafRef.current = requestAnimationFrame(apply)
  }, [apply])

  const setViewNow = useCallback(
    (next: View) => {
      viewRef.current = next
      setView(next)
      schedule()
    },
    [schedule],
  )

  /** Scale the diagram to fit, so a wide schema is not off-screen on open. */
  const fitToFrame = useCallback(() => {
    const frame = frameRef.current
    if (!frame || !diagram || diagram.width === 0) return
    const z = Math.min(
      1,
      Math.max(
        MIN_ZOOM,
        Math.min((frame.clientWidth - PAD) / diagram.width, (frame.clientHeight - PAD) / diagram.height),
      ),
    )
    // Centre, rather than pinning to a corner and leaving the rest empty.
    setViewNow({
      x: Math.max(PAD / 2, (frame.clientWidth - diagram.width * z) / 2),
      y: Math.max(PAD / 2, (frame.clientHeight - diagram.height * z) / 2),
      z,
    })
  }, [diagram, setViewNow])

  useEffect(() => {
    fitToFrame()
  }, [fitToFrame])

  // A new layout invalidates hand-placed offsets: they were relative to
  // coordinates that no longer exist.
  // biome-ignore lint/correctness/useExhaustiveDependencies: keyed on the layout, not the object identity
  useEffect(() => {
    setOffsets({})
  }, [focus, depth])

  useEffect(
    () => () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current)
    },
    [],
  )

  /**
   * Zoom on wheel.
   *
   * Bound natively with `{ passive: false }` rather than through React's
   * `onWheel`. React attaches wheel handlers passively, where
   * `preventDefault()` is a no-op — so the page scrolled underneath the
   * diagram instead of the diagram zooming.
   */
  useEffect(() => {
    const frame = frameRef.current
    if (!frame) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const rect = frame.getBoundingClientRect()
      const v = viewRef.current
      const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.z * (e.deltaY < 0 ? 1.12 : 1 / 1.12)))
      // Keep the point under the cursor fixed while zooming.
      const cx = e.clientX - rect.left
      const cy = e.clientY - rect.top
      const k = next / v.z
      setViewNow({ x: cx - (cx - v.x) * k, y: cy - (cy - v.y) * k, z: next })
    }
    frame.addEventListener('wheel', onWheel, { passive: false })
    return () => frame.removeEventListener('wheel', onWheel)
  }, [setViewNow])

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return
    dragRef.current = { px: e.clientX, py: e.clientY }
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  }

  const onPointerMove = (e: React.PointerEvent) => {
    const d = dragRef.current
    if (!d) return
    const v = viewRef.current
    viewRef.current = { ...v, x: v.x + (e.clientX - d.px), y: v.y + (e.clientY - d.py) }
    dragRef.current = { px: e.clientX, py: e.clientY }
    schedule()
  }

  const onPointerUp = (e: React.PointerEvent) => {
    if (!dragRef.current) return
    dragRef.current = null
    ;(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId)
    // Commit once, at the end, rather than on every frame.
    setView(viewRef.current)
  }

  /**
   * Drag one node. Offsets are divided by the zoom so a box tracks the cursor
   * at any scale — without that, dragging at 40% moves the node 2.5x too far.
   */
  const startNodeDrag = (e: React.PointerEvent, table: string) => {
    if (e.button !== 0) return
    e.stopPropagation()
    nodeDragRef.current = { table, px: e.clientX, py: e.clientY }
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  }

  const moveNode = (e: React.PointerEvent) => {
    const d = nodeDragRef.current
    if (!d) return
    e.stopPropagation()
    const z = viewRef.current.z || 1
    const dx = (e.clientX - d.px) / z
    const dy = (e.clientY - d.py) / z
    nodeDragRef.current = { ...d, px: e.clientX, py: e.clientY }
    setOffsets((prev) => {
      const at = prev[d.table] ?? { dx: 0, dy: 0 }
      return { ...prev, [d.table]: { dx: at.dx + dx, dy: at.dy + dy } }
    })
  }

  const endNodeDrag = (e: React.PointerEvent) => {
    if (!nodeDragRef.current) return
    nodeDragRef.current = null
    ;(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId)
  }

  const matches = useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q || !diagram) return null
    return new Set(diagram.nodes.filter((n) => n.table.toLowerCase().includes(q)).map((n) => n.table))
  }, [search, diagram])

  if (read.error) {
    return (
      <StatusPanel
        variant="alert"
        icon={<AlertCircle size={18} />}
        headline="could not lay out the schema"
        detail={read.error}
      />
    )
  }
  if (read.loading && !diagram) {
    return <div className="db-msg db-pulse">· laying out the schema…</div>
  }
  if (!diagram || diagram.nodes.length === 0) {
    return (
      <EmptyState
        icon={TableIcon}
        title="nothing to diagram"
        description="this database has no tables yet. create one and refresh."
      />
    )
  }

  const related = diagram.nodes.length - diagram.isolated.length

  return (
    <div className="db-erd">
      <div className="db-erd-bar">
        <span className="db-erd-stat">
          {diagram.nodes.length} tables · {diagram.edges.length} relations
        </span>
        {related === 0 ? (
          <span className="db-erd-stat quiet">no foreign keys — nothing to connect</span>
        ) : (
          <span className="db-erd-stat quiet">
            {diagram.isolated.length} unrelated · {diagram.crossings} crossings
          </span>
        )}
        {diagram.truncated ? <Badge variant="warn">truncated</Badge> : null}
        <div className="db-erd-spacer" />
        {focus ? (
          <>
            <span className="db-erd-focus">
              focused on <strong>{focus}</strong>
            </span>
            <Select
              value={String(depth)}
              onChange={(d) => setDepth(Number(d))}
              options={[
                { value: '1', label: 'direct relations' },
                { value: '2', label: '2 hops' },
                { value: '3', label: '3 hops' },
              ]}
              aria-label="how far to expand"
            />
            <Button variant="ghost" size="sm" onClick={() => setFocus(null)}>
              whole schema
            </Button>
          </>
        ) : null}
        <Input
          value={search}
          onChange={setSearch}
          placeholder="find a table"
          preserveCase
          className="db-erd-search"
          aria-label="find a table"
        />
        <span className="db-erd-zoom">{Math.round(view.z * 100)}%</span>
        {Object.keys(offsets).length > 0 ? (
          <Button variant="ghost" size="sm" onClick={() => setOffsets({})}>
            reset positions
          </Button>
        ) : null}
        <Button variant="ghost" size="sm" onClick={fitToFrame}>
          <RefreshCw size={13} aria-hidden />
          fit
        </Button>
      </div>

      {/* What was left out. A diagram that simply stops looks complete; naming
          the next ring is what makes expanding a decision rather than a guess. */}
      {focus && diagram.frontier.length > 0 ? (
        <div className="db-erd-frontier">
          <span className="db-erd-frontier-label">also connected:</span>
          {diagram.frontier.map((t) => (
            <button key={t} type="button" className="db-erd-frontier-item" onClick={() => setFocus(t)}>
              {t}
            </button>
          ))}
          <button type="button" className="db-erd-frontier-more" onClick={() => setDepth((d) => d + 1)}>
            pull them in
          </button>
        </div>
      ) : null}

      <div
        className="db-erd-frame"
        ref={frameRef}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        role="application"
        aria-label="schema diagram — drag to pan, scroll to zoom"
      >
        <div
          className="db-erd-canvas"
          ref={canvasRef}
          style={{
            width: diagram.width,
            height: diagram.height,
            transform: `translate(${view.x}px, ${view.y}px) scale(${view.z})`,
          }}
        >
          {/* Component boundaries sit behind everything. A schema is usually
              several independent clusters, and drawing that fact is what lets
              a reader take in the structure before reading a single name. */}
          {diagram.components
            .filter((c) => c.tables.length > 1)
            .map((c) => (
              <div
                key={c.index}
                className="db-erd-group"
                style={{
                  left: c.x - GROUP_PAD,
                  top: c.y - GROUP_PAD,
                  width: c.w + GROUP_PAD * 2,
                  height: c.h + GROUP_PAD * 2,
                }}
              >
                <span className="db-erd-group-tag">
                  {c.hub ? `${c.hub} · ` : ''}
                  {c.tables.length} related
                </span>
              </div>
            ))}

          <svg className="db-erd-edges" width={diagram.width} height={diagram.height} aria-hidden>
            <title>foreign key relationships</title>
            {diagram.edges.map((e) => {
              const dim = matches ? !matches.has(e.from) && !matches.has(e.to) : false
              const hot = selected === e.from || selected === e.to
              // An edge has to follow the nodes it joins. The worker routed it
              // against the original coordinates, so each end is shifted by
              // whatever its own node was dragged and the corner between them
              // is interpolated — the elbow stays orthogonal without
              // re-running the router in the browser.
              const from = offsets[e.from]
              const to = offsets[e.to]
              const points = shiftEdge(e.points, from, to)
              const a = points[0]
              const b = points[points.length - 1]
              return (
                <g
                  key={`${e.from}.${e.from_column}->${e.to}.${e.to_column}`}
                  className={`db-erd-edge${hot ? ' hot' : ''}${dim ? ' dim' : ''}`}
                >
                  <polyline points={points.map((p) => `${p.x},${p.y}`).join(' ')} fill="none" />
                  {/* Endpoint dots, the idiom both reference clients use: the
                      line lands on the exact column row, so a relationship
                      reads as column-to-column rather than box-to-box. */}
                  <circle className="db-erd-endpoint" cx={a.x} cy={a.y} r={3} />
                  <circle className="db-erd-endpoint target" cx={b.x} cy={b.y} r={3} />
                </g>
              )
            })}
          </svg>

          {diagram.nodes.map((n) => {
            const dim = matches ? !matches.has(n.table) : false
            // Hub emphasis by border weight, not fill — the palette stays rationed.
            const hub = n.degree >= 5 ? ' hub-3' : n.degree >= 2 ? ' hub-2' : ''
            const off = offsets[n.table]
            const isFocus = diagram.focus === n.table
            return (
              <div
                key={n.table}
                className={[
                  'db-erd-node',
                  hub.trim(),
                  selected === n.table ? 'selected' : '',
                  isFocus ? 'focused' : '',
                  dim ? 'dim' : '',
                  off ? 'moved' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
                style={{
                  left: n.x + (off?.dx ?? 0),
                  top: n.y + (off?.dy ?? 0),
                  width: n.w,
                  minHeight: n.h,
                }}
              >
                {/* The header is the drag handle as well as the button: one
                    place to grab, and a click that never moved still selects. */}
                <button
                  type="button"
                  className="db-erd-node-head"
                  onPointerDown={(e) => startNodeDrag(e, n.table)}
                  onPointerMove={moveNode}
                  onPointerUp={endNodeDrag}
                  onPointerCancel={endNodeDrag}
                  onClick={() => setSelected(selected === n.table ? null : n.table)}
                  onDoubleClick={() => {
                    setFocus(n.table)
                    setSelected(null)
                  }}
                  title={`${n.table} · ${n.degree} relation${n.degree === 1 ? '' : 's'} · drag to move, double-click to focus`}
                >
                  <span className="db-erd-node-name">{n.table}</span>
                  {n.degree > 0 ? <span className="db-erd-node-deg">{n.degree}</span> : null}
                </button>
                <div className="db-erd-cols">
                  {n.columns.map((c) => (
                    <div className="db-erd-col" key={c.name}>
                      {c.primary_key ? (
                        <KeyRound size={9} className="db-erd-glyph pk" aria-label="primary key" />
                      ) : c.foreign_key ? (
                        <Link2 size={9} className="db-erd-glyph fk" aria-label="foreign key" />
                      ) : (
                        <span className="db-erd-glyph" />
                      )}
                      <span className="db-erd-col-name">{c.name}</span>
                      {/* Required columns carry a marker rather than the
                          nullable ones: in most schemas NOT NULL is the
                          smaller set, so it is the one worth marking. */}
                      {!c.nullable && !c.primary_key ? (
                        <span className="db-erd-req" title="not null">
                          *
                        </span>
                      ) : null}
                      <span className="db-erd-col-type">{c.type.toLowerCase()}</span>
                    </div>
                  ))}
                  {n.hidden_columns > 0 ? <div className="db-erd-col more">+{n.hidden_columns} more</div> : null}
                </div>
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
