import { StatusDot } from '@iii-dev/console-ui'
import { type KeyboardEvent, useMemo } from 'react'
import { Layers } from './icons'
import { layoutTopology, related, type TopologyContainer, type TopologyInput } from './topology-layout'

interface TopologyProps {
  input: TopologyInput
  selected: string | null
  onSelect: (name: string | null) => void
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

function edgePath(edge: { start: { x: number; y: number }; end: { x: number; y: number } }): string {
  const dx = Math.max(32, (edge.end.x - edge.start.x) / 2)
  return `M ${edge.start.x} ${edge.start.y} C ${edge.start.x + dx} ${edge.start.y}, ${edge.end.x - dx} ${edge.end.y}, ${edge.end.x - 1} ${edge.end.y}`
}

export function Topology({ input, selected, onSelect }: TopologyProps) {
  const layout = useMemo(() => layoutTopology(input), [input])
  const relation = useMemo(() => (selected ? related(input.containers, selected) : null), [input.containers, selected])

  const order = layout.nodes.map((n) => n.container.name)
  const moveFocus = (event: KeyboardEvent<HTMLButtonElement>, name: string) => {
    const index = order.indexOf(name)
    if (index < 0) return
    let next: number | null = null
    if (event.key === 'ArrowDown') next = Math.min(order.length - 1, index + 1)
    else if (event.key === 'ArrowUp') next = Math.max(0, index - 1)
    else if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
      const current = layout.nodes[index]
      const dir = event.key === 'ArrowRight' ? 1 : -1
      const candidates = layout.nodes
        .map((n, i) => ({ n, i }))
        .filter(({ n }) => n.level === current.level + dir)
        .sort((a, b) => Math.abs(a.n.y - current.y) - Math.abs(b.n.y - current.y))
      next = candidates[0]?.i ?? null
    } else if (event.key === 'Escape') {
      onSelect(null)
      return
    }
    if (next === null) return
    event.preventDefault()
    const target = event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`[data-node="${order[next]}"]`)
    target?.focus()
  }

  if (layout.nodes.length === 0) return null

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

  const edges = [...layout.edges].sort((a, b) => {
    const rank = (rel: string | undefined) => (rel === 'up' || rel === 'down' ? 1 : 0)
    return rank(edgeState(a.from, a.to)) - rank(edgeState(b.from, b.to))
  })

  return (
    <div className="cu-topo-scroll">
      <div className="cu-topo" style={{ width: layout.width, height: layout.height }}>
        <svg aria-hidden="true" className="cu-topo-svg" width={layout.width} height={layout.height}>
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
          {edges.map((edge) => (
            <path
              key={edge.key}
              className="cu-topo-edge"
              data-rel={edgeState(edge.from, edge.to)}
              data-running={input.containers.find((c) => c.name === edge.to)?.state === 'ready' ? '' : undefined}
              d={edgePath(edge)}
              markerEnd="url(#cu-topo-arrow)"
            />
          ))}
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
          style={{ left: layout.engine.x, top: layout.engine.y, width: layout.engine.w, height: layout.engine.h }}
        >
          <span className="cu-topo-title">
            <Layers />
            engine
          </span>
          <span className="cu-mono cu-topo-sub">
            {input.engine.host ?? '127.0.0.1'}
            {input.engine.port ? `:${input.engine.port}` : ''}
          </span>
          <span className="cu-topo-sub">{input.engine.pid ? `daemon pid ${input.engine.pid}` : 'compose daemon'}</span>
        </div>

        {layout.nodes.map((node) => {
          const c = node.container
          return (
            <button
              key={c.name}
              type="button"
              data-node={c.name}
              data-state={c.state}
              data-rel={nodeState(c.name)}
              aria-pressed={selected === c.name}
              className="cu-topo-node"
              style={{ left: node.x, top: node.y, width: node.w, height: node.h }}
              onClick={() => onSelect(selected === c.name ? null : c.name)}
              onKeyDown={(event) => moveFocus(event, c.name)}
              title={c.lastError ?? undefined}
            >
              <span className="cu-topo-title">
                <StatusDot tone={dotTone(c.state)} pulse={c.state === 'starting'} aria-hidden />
                <span className="cu-mono cu-topo-name">{c.name}</span>
                {c.ports.length ? <span className="cu-mono cu-topo-ports">:{c.ports.join(' :')}</span> : null}
              </span>
              <span className="cu-topo-sub" data-state={c.state}>
                {c.state === 'ready' ? `pid ${c.pid ?? '–'}` : c.state}
                {sourceLabel(c) ? (
                  <span className="cu-mono cu-topo-source" title={c.ref ?? undefined}>
                    {sourceLabel(c)}
                  </span>
                ) : null}
              </span>
            </button>
          )
        })}
      </div>
    </div>
  )
}
