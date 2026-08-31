import { Button, StatusDot } from '@iii-dev/console-ui'
import { type KeyboardEvent, type PointerEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Layers } from './icons'
import { type Box, layoutTopology, related, TOPO, type TopologyContainer, type TopologyInput } from './topology-layout'

interface TopologyProps {
  input: TopologyInput
  selected: string | null
  onSelect: (name: string | null) => void
}

type Offset = { x: number; y: number }
type Offsets = Record<string, Offset>

const ENGINE = 'engine'
const DRAG_THRESHOLD = 3
const NUDGE = 8
const STORAGE_PREFIX = 'compose-ui:topology:'
const STACK_BELOW = 560

function storageKey(input: TopologyInput): string {
  return `${STORAGE_PREFIX}${input.namespace ?? ''}:${input.file ?? ''}`
}

function readOffsets(key: string): Offsets {
  try {
    const raw = window.localStorage.getItem(key)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object') return {}
    const out: Offsets = {}
    for (const [name, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (value && typeof value === 'object') {
        const { x, y } = value as { x?: unknown; y?: unknown }
        if (typeof x === 'number' && typeof y === 'number') out[name] = { x, y }
      }
    }
    return out
  } catch {
    return {}
  }
}

function writeOffsets(key: string, offsets: Offsets) {
  try {
    if (Object.keys(offsets).length === 0) window.localStorage.removeItem(key)
    else window.localStorage.setItem(key, JSON.stringify(offsets))
  } catch {}
}

function dotTone(state: string): 'accent' | 'alert' | 'warn' | 'ink' {
  if (state === 'ready') return 'accent'
  if (state === 'failed') return 'alert'
  if (state === 'starting') return 'warn'
  return 'ink'
}

function sourceLabel(c: TopologyContainer): string | null {
  if (c.source === 'package') return `${(c.ref ?? '').split('/').pop() ?? ''}${c.version ? `@${c.version}` : ''}`
  if (c.source === 'path') {
    const ref = c.ref ?? ''
    if (ref === '.' || ref === './' || ref === `../${c.name}` || ref === `./${c.name}`) return null
    const parts = ref.split('/').filter(Boolean)
    return parts.length > 2 ? `…/${parts.slice(-2).join('/')}` : ref
  }
  return c.ref || null
}

function edgePath(from: Box, to: Box): string {
  const sx = from.x + from.w
  const sy = from.y + from.h / 2
  const ex = to.x
  const ey = to.y + to.h / 2
  const dx = Math.max(32, Math.abs(ex - sx) / 2)
  return `M ${sx} ${sy} C ${sx + dx} ${sy}, ${ex - dx} ${ey}, ${ex - 1} ${ey}`
}

function shifted(box: Box, offset: Offset | undefined): Box {
  if (!offset) return box
  return { ...box, x: box.x + offset.x, y: box.y + offset.y }
}

type Drag = { name: string; pointerId: number; startX: number; startY: number; origin: Offset; moved: boolean }

export function Topology({ input, selected, onSelect }: TopologyProps) {
  const layout = useMemo(() => layoutTopology(input), [input])
  const relation = useMemo(() => (selected ? related(input.containers, selected) : null), [input.containers, selected])
  const key = storageKey(input)
  const [offsets, setOffsets] = useState<Offsets>(() => readOffsets(key))
  const keyRef = useRef(key)
  useEffect(() => {
    if (keyRef.current === key) return
    keyRef.current = key
    setOffsets(readOffsets(key))
  }, [key])

  const dragRef = useRef<Drag | null>(null)
  const [dragging, setDragging] = useState<string | null>(null)
  const suppressClickRef = useRef(false)
  const frameRef = useRef<HTMLDivElement>(null)
  const [stacked, setStacked] = useState(false)
  useEffect(() => {
    const frame = frameRef.current
    if (!frame || typeof ResizeObserver === 'undefined') return
    const update = (width: number) => {
      if (width > 0) setStacked((current) => (current === width < STACK_BELOW ? current : width < STACK_BELOW))
    }
    update(frame.getBoundingClientRect().width)
    const observer = new ResizeObserver(([entry]) => {
      if (entry) update(entry.contentRect.width)
    })
    observer.observe(frame)
    return () => observer.disconnect()
  }, [])

  const commit = useCallback(
    (next: Offsets) => {
      setOffsets(next)
      writeOffsets(key, next)
    },
    [key],
  )

  const boxes = useMemo(() => {
    const map = new Map<string, Box>()
    map.set(ENGINE, shifted(layout.engine, offsets[ENGINE]))
    for (const node of layout.nodes) map.set(node.container.name, shifted(node, offsets[node.container.name]))
    return map
  }, [layout, offsets])

  const bounds = useMemo(() => {
    let width = layout.width
    let height = layout.height
    for (const box of boxes.values()) {
      width = Math.max(width, box.x + box.w + TOPO.pad)
      height = Math.max(height, box.y + box.h + TOPO.pad)
    }
    return { width, height }
  }, [boxes, layout])

  const baseBox = useCallback(
    (name: string): Box | undefined =>
      name === ENGINE ? layout.engine : layout.nodes.find((n) => n.container.name === name),
    [layout],
  )

  const clamp = useCallback(
    (name: string, offset: Offset): Offset => {
      const base = baseBox(name)
      if (!base) return offset
      return { x: Math.max(TOPO.pad - base.x, offset.x), y: Math.max(TOPO.pad - base.y, offset.y) }
    },
    [baseBox],
  )

  const order = layout.nodes.map((n) => n.container.name)

  const onKeyDown = (event: KeyboardEvent<HTMLElement>, name: string) => {
    const arrows: Record<string, [number, number]> = {
      ArrowUp: [0, -1],
      ArrowDown: [0, 1],
      ArrowLeft: [-1, 0],
      ArrowRight: [1, 0],
    }
    const arrow = arrows[event.key]
    if (arrow && event.shiftKey) {
      event.preventDefault()
      const current = offsets[name] ?? { x: 0, y: 0 }
      commit({ ...offsets, [name]: clamp(name, { x: current.x + arrow[0] * NUDGE, y: current.y + arrow[1] * NUDGE }) })
      return
    }
    if (event.key === 'Escape') {
      onSelect(null)
      return
    }
    if (!arrow || name === ENGINE) return
    const index = order.indexOf(name)
    if (index < 0) return
    let next: number | null = null
    if (event.key === 'ArrowDown') next = Math.min(order.length - 1, index + 1)
    else if (event.key === 'ArrowUp') next = Math.max(0, index - 1)
    else {
      const current = layout.nodes[index]
      const dir = event.key === 'ArrowRight' ? 1 : -1
      const candidates = layout.nodes
        .map((n, i) => ({ n, i }))
        .filter(({ n }) => n.level === current.level + dir)
        .sort((a, b) => Math.abs(a.n.y - current.y) - Math.abs(b.n.y - current.y))
      next = candidates[0]?.i ?? null
    }
    if (next === null) return
    event.preventDefault()
    const target = event.currentTarget.parentElement?.querySelector<HTMLElement>(`[data-node="${order[next]}"]`)
    target?.focus()
  }

  const onPointerDown = (event: PointerEvent<HTMLElement>, name: string) => {
    if (event.button !== 0) return
    dragRef.current = {
      name,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      origin: offsets[name] ?? { x: 0, y: 0 },
      moved: false,
    }
    try {
      event.currentTarget.setPointerCapture(event.pointerId)
    } catch {}
  }

  const onPointerMove = (event: PointerEvent<HTMLElement>) => {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== event.pointerId) return
    const dx = event.clientX - drag.startX
    const dy = event.clientY - drag.startY
    if (!drag.moved && Math.hypot(dx, dy) < DRAG_THRESHOLD) return
    if (!drag.moved) {
      drag.moved = true
      setDragging(drag.name)
    }
    const next = clamp(drag.name, { x: drag.origin.x + dx, y: drag.origin.y + dy })
    setOffsets((prev) => ({ ...prev, [drag.name]: next }))
  }

  const endDrag = (event: PointerEvent<HTMLElement>, cancelled: boolean) => {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== event.pointerId) return
    dragRef.current = null
    setDragging(null)
    try {
      event.currentTarget.releasePointerCapture(event.pointerId)
    } catch {}
    if (!drag.moved) return
    suppressClickRef.current = true
    if (cancelled) {
      setOffsets((prev) => ({ ...prev, [drag.name]: drag.origin }))
      return
    }
    setOffsets((prev) => {
      writeOffsets(key, prev)
      return prev
    })
  }

  const edgeState = (from: string, to: string): string | undefined => {
    if (!selected || !relation) return undefined
    if (from === selected || relation.upstream.has(from)) {
      if (to === selected || relation.upstream.has(to)) return 'up'
    }
    if (from === selected || relation.downstream.has(from)) {
      if (relation.downstream.has(to)) return 'down'
    }
    return 'dim'
  }

  const nodeState = (name: string): string | undefined => {
    if (!selected || !relation) return undefined
    if (name === selected) return 'selected'
    if (relation.upstream.has(name)) return 'up'
    if (relation.downstream.has(name)) return 'down'
    return 'dim'
  }

  if (layout.nodes.length === 0) return null

  const rank = (rel: string | undefined) => (rel === 'up' || rel === 'down' ? 1 : 0)
  const edges = [...layout.edges].sort((a, b) => rank(edgeState(a.from, a.to)) - rank(edgeState(b.from, b.to)))
  const moved = Object.keys(offsets).length > 0
  const dragProps = (name: string) => ({
    onPointerDown: (event: PointerEvent<HTMLElement>) => onPointerDown(event, name),
    onPointerMove,
    onPointerUp: (event: PointerEvent<HTMLElement>) => endDrag(event, false),
    onPointerCancel: (event: PointerEvent<HTMLElement>) => endDrag(event, true),
    onLostPointerCapture: (event: PointerEvent<HTMLElement>) => endDrag(event, false),
  })
  const engineBox = boxes.get(ENGINE) ?? layout.engine

  const card = (c: TopologyContainer, box: Box | null) => {
    const source = sourceLabel(c)
    return (
      <button
        key={c.name}
        type="button"
        data-node={c.name}
        data-state={c.state}
        data-rel={nodeState(c.name)}
        data-dragging={dragging === c.name ? '' : undefined}
        aria-pressed={selected === c.name}
        className="cu-topo-node"
        style={box ? { left: box.x, top: box.y, width: box.w, height: box.h } : undefined}
        onClick={() => {
          if (suppressClickRef.current) {
            suppressClickRef.current = false
            return
          }
          onSelect(selected === c.name ? null : c.name)
        }}
        onKeyDown={(event) => onKeyDown(event, c.name)}
        title={c.lastError ?? undefined}
        {...(box ? dragProps(c.name) : {})}
      >
        <span className="cu-topo-title">
          <StatusDot tone={dotTone(c.state)} pulse={c.state === 'starting'} aria-hidden />
          <span className="cu-mono cu-topo-name">{c.name}</span>
          {c.ports.length ? <span className="cu-mono cu-topo-ports">:{c.ports.join(' :')}</span> : null}
        </span>
        <span className="cu-topo-sub" data-state={c.state}>
          {c.state === 'ready' ? `pid ${c.pid ?? '–'}` : c.state}
          {source ? (
            <span className="cu-mono cu-topo-source" title={c.ref ?? undefined}>
              {source}
            </span>
          ) : null}
        </span>
      </button>
    )
  }

  const levels = new Map<number, TopologyContainer[]>()
  for (const node of layout.nodes) {
    const list = levels.get(node.level)
    if (list) list.push(node.container)
    else levels.set(node.level, [node.container])
  }

  const stack = (
    <div className="cu-topo-stack">
      <div className="cu-topo-engine cu-topo-engine-static">
        <span className="cu-topo-title">
          <Layers />
          engine
        </span>
        <span className="cu-mono cu-topo-sub">
          {input.engine.host ?? '127.0.0.1'}
          {input.engine.port ? `:${input.engine.port}` : ''}
          {input.engine.pid ? ` · daemon pid ${input.engine.pid}` : ''}
        </span>
      </div>
      <span className="cu-topo-group-label cu-topo-group-label-static">
        <span className="cu-topo-group-kind">namespace</span>
        <span className="cu-mono">{input.namespace ?? '–'}</span>
      </span>
      {[...levels.entries()]
        .sort((a, b) => a[0] - b[0])
        .map(([level, containers]) => (
          <section key={level} className="cu-topo-level">
            <h3 className="cu-topo-level-head">
              {level === 0 ? 'Starts with the engine' : `Start level ${level}`}
              <span className="cu-topo-level-count">{containers.length}</span>
            </h3>
            <div className="cu-topo-level-grid">{containers.map((c) => card(c, null))}</div>
          </section>
        ))}
    </div>
  )

  const canvas = (
    <>
      <div className="cu-topo-toolbar">
        <span className="cu-topo-hint">Drag a card to rearrange; Shift+arrows nudge the focused one.</span>
        {moved ? (
          <Button variant="ghost" size="sm" onClick={() => commit({})}>
            Reset layout
          </Button>
        ) : null}
      </div>
      <div className="cu-topo-scroll">
        <div
          className="cu-topo"
          style={{ width: bounds.width, height: bounds.height }}
          data-dragging={dragging ? '' : undefined}
        >
          <svg aria-hidden="true" className="cu-topo-svg" width={bounds.width} height={bounds.height}>
            <defs>
              <marker
                id="cu-topo-arrow"
                viewBox="0 0 8 8"
                refX="7"
                refY="4"
                markerWidth="8"
                markerHeight="8"
                orient="auto"
              >
                <path d="M0 0 L8 4 L0 8 Z" />
              </marker>
            </defs>
            {edges.map((edge) => {
              const from = boxes.get(edge.from)
              const to = boxes.get(edge.to)
              if (!from || !to) return null
              return (
                <path
                  key={edge.key}
                  className="cu-topo-edge"
                  data-rel={edgeState(edge.from, edge.to)}
                  data-running={input.containers.find((c) => c.name === edge.to)?.state === 'ready' ? '' : undefined}
                  d={edgePath(from, to)}
                  markerEnd="url(#cu-topo-arrow)"
                />
              )
            })}
          </svg>

          <div
            className="cu-topo-group"
            style={{ left: layout.group.x, top: layout.group.y, width: layout.group.w, height: layout.group.h }}
          >
            <span className="cu-topo-group-label">
              <span className="cu-topo-group-kind">namespace</span>
              <span className="cu-mono">{input.namespace ?? '–'}</span>
              {input.file ? <span className="cu-topo-group-file cu-mono">{input.file.split('/').pop()}</span> : null}
            </span>
          </div>

          <div
            className="cu-topo-engine"
            data-node={ENGINE}
            data-dragging={dragging === ENGINE ? '' : undefined}
            style={{ left: engineBox.x, top: engineBox.y, width: engineBox.w, height: engineBox.h }}
            {...dragProps(ENGINE)}
          >
            <span className="cu-topo-title">
              <Layers />
              engine
            </span>
            <span className="cu-mono cu-topo-sub">
              {input.engine.host ?? '127.0.0.1'}
              {input.engine.port ? `:${input.engine.port}` : ''}
            </span>
            <span className="cu-topo-sub">
              {input.engine.pid ? `daemon pid ${input.engine.pid}` : 'compose daemon'}
            </span>
          </div>

          {layout.nodes.map((node) => card(node.container, boxes.get(node.container.name) ?? node))}
        </div>
      </div>
    </>
  )

  return (
    <div className="cu-topo-frame" ref={frameRef}>
      {stacked ? stack : canvas}
    </div>
  )
}
