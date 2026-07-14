import { Pin, PinOff, Trash2, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import type { MemoryFact } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * The bank as a schematic constellation: entity hubs (squares) with their
 * facts as spokes (circles), multi-entity facts bridging hubs. A
 * render-time projection of the flat store — the `entities[]` field IS the
 * edge list; storage stays flat. Pinned facts render in the accent color;
 * click a node to inspect, pin, or tombstone it in place. Layout is
 * deterministic (golden-angle spiral over hubs sorted by degree), so the
 * same bank always draws the same map.
 */

interface MemoryGraphProps {
  facts: MemoryFact[]
  onPin: (fact: MemoryFact) => void
  onDelete: (fact: MemoryFact) => void
  busy: boolean
}

interface HubNode {
  entity: string
  x: number
  y: number
  count: number
}

interface FactNode {
  fact: MemoryFact
  x: number
  y: number
}

interface Edge {
  x1: number
  y1: number
  x2: number
  y2: number
  factId: string
  entity: string
}

interface Layout {
  hubs: HubNode[]
  nodes: FactNode[]
  edges: Edge[]
  viewBox: string
}

const GOLDEN_ANGLE = 2.399963
const NO_ENTITY_HUB = '(no entities)'

function layoutGraph(facts: MemoryFact[]): Layout {
  const byEntity = new Map<string, MemoryFact[]>()
  for (const fact of facts) {
    const keys = fact.entities.length > 0 ? fact.entities : [NO_ENTITY_HUB]
    for (const entity of keys) {
      const list = byEntity.get(entity) ?? []
      list.push(fact)
      byEntity.set(entity, list)
    }
  }

  // Hubs on a golden-angle spiral, densest first so big clusters sit
  // center. Spiral spacing grows with cluster size to avoid overlap.
  const hubEntries = [...byEntity.entries()].sort(
    (a, b) => b[1].length - a[1].length || a[0].localeCompare(b[0]),
  )
  const hubs: HubNode[] = []
  const hubPos = new Map<string, { x: number; y: number }>()
  hubEntries.forEach(([entity, list], i) => {
    const r = 90 * Math.sqrt(i + (i === 0 ? 0 : 1.6))
    const theta = i * GOLDEN_ANGLE
    const x = Math.round(r * Math.cos(theta))
    const y = Math.round(r * Math.sin(theta))
    hubs.push({ entity, x, y, count: list.length })
    hubPos.set(entity, { x, y })
  })

  const nodes: FactNode[] = []
  const edges: Edge[] = []
  const placed = new Map<string, { x: number; y: number }>()

  // Single-entity facts ring their hub; multi-entity facts sit at the
  // centroid of their hubs (nudged per-index so siblings don't stack).
  const ringIndex = new Map<string, number>()
  for (const fact of facts) {
    const keys = fact.entities.length > 0 ? fact.entities : [NO_ENTITY_HUB]
    let x: number
    let y: number
    if (keys.length === 1) {
      const hub = hubPos.get(keys[0])
      if (!hub) continue
      const siblings = byEntity.get(keys[0])?.length ?? 1
      const idx = ringIndex.get(keys[0]) ?? 0
      ringIndex.set(keys[0], idx + 1)
      const ring = 34 + 10 * Math.floor(idx / 14)
      const angle = (idx % 14) * ((2 * Math.PI) / Math.min(siblings, 14))
      x = hub.x + Math.round(ring * Math.cos(angle))
      y = hub.y + Math.round(ring * Math.sin(angle))
    } else {
      const points = keys
        .map((k) => hubPos.get(k))
        .filter((p): p is { x: number; y: number } => p !== undefined)
      const cx = points.reduce((s, p) => s + p.x, 0) / points.length
      const cy = points.reduce((s, p) => s + p.y, 0) / points.length
      const nudge = (placed.size % 5) - 2
      x = Math.round(cx + nudge * 9)
      y = Math.round(cy + nudge * 6)
    }
    placed.set(fact.id, { x, y })
    nodes.push({ fact, x, y })
    for (const entity of keys) {
      const hub = hubPos.get(entity)
      if (hub) {
        edges.push({
          x1: hub.x,
          y1: hub.y,
          x2: x,
          y2: y,
          factId: fact.id,
          entity,
        })
      }
    }
  }

  const xs = [...hubs.map((h) => h.x), ...nodes.map((n) => n.x)]
  const ys = [...hubs.map((h) => h.y), ...nodes.map((n) => n.y)]
  const pad = 70
  const minX = Math.min(0, ...xs) - pad
  const minY = Math.min(0, ...ys) - pad
  const maxX = Math.max(0, ...xs) + pad
  const maxY = Math.max(0, ...ys) + pad

  return {
    hubs,
    nodes,
    edges,
    viewBox: `${minX} ${minY} ${maxX - minX} ${maxY - minY}`,
  }
}

export function MemoryGraph({
  facts,
  onPin,
  onDelete,
  busy,
}: MemoryGraphProps) {
  const live = useMemo(() => facts.filter((f) => f.invalid_at == null), [facts])
  const layout = useMemo(() => layoutGraph(live), [live])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [hoverId, setHoverId] = useState<string | null>(null)

  const selected = live.find((f) => f.id === selectedId) ?? null
  const activeId = selectedId ?? hoverId

  if (live.length === 0) {
    return (
      <EmptyState
        title="nothing to map yet"
        description="the graph draws entity hubs with their facts as spokes. save or extract a few facts with entities and they appear here."
      />
    )
  }

  return (
    <div className="relative flex-1 min-h-0 border border-rule">
      <svg
        viewBox={layout.viewBox}
        className="w-full h-full min-h-[420px]"
        role="img"
        aria-label="memory graph: entity hubs and fact nodes"
      >
        {layout.edges.map((edge) => (
          <line
            key={`${edge.factId}:${edge.entity}`}
            x1={edge.x1}
            y1={edge.y1}
            x2={edge.x2}
            y2={edge.y2}
            className={cn(
              'stroke-rule',
              activeId === edge.factId && 'stroke-accent',
            )}
            strokeWidth={activeId === edge.factId ? 1.6 : 1}
          />
        ))}
        {layout.hubs.map((hub) => (
          <g key={hub.entity}>
            <rect
              x={hub.x - 4}
              y={hub.y - 4}
              width={8}
              height={8}
              className="fill-ink-faint"
            />
            <text
              x={hub.x}
              y={hub.y + 20}
              textAnchor="middle"
              className="fill-ink-faint font-mono lowercase"
              fontSize={11}
            >
              {hub.entity} ({hub.count})
            </text>
          </g>
        ))}
        {layout.nodes.map(({ fact, x, y }) => (
          // biome-ignore lint/a11y/useSemanticElements: SVG nodes act as buttons
          <g
            key={fact.id}
            role="button"
            tabIndex={0}
            aria-label={`fact: ${fact.text.slice(0, 60)}`}
            className="cursor-pointer focus:outline-none"
            onClick={() =>
              setSelectedId((cur) => (cur === fact.id ? null : fact.id))
            }
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                setSelectedId((cur) => (cur === fact.id ? null : fact.id))
              }
            }}
            onMouseEnter={() => setHoverId(fact.id)}
            onMouseLeave={() => setHoverId(null)}
          >
            <circle
              cx={x}
              cy={y}
              r={activeId === fact.id ? 7 : 5}
              className={cn(
                fact.pinned ? 'fill-accent' : 'fill-ink',
                activeId === fact.id && 'stroke-accent',
              )}
              strokeWidth={1.5}
            />
          </g>
        ))}
      </svg>

      <div className="absolute bottom-2 left-2 flex items-center gap-3 font-mono text-[10px] lowercase text-ink-ghost bg-bg/90 border border-rule-2 px-2 py-1">
        <span className="flex items-center gap-1">
          <span className="inline-block w-2 h-2 bg-ink-faint" /> entity
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block w-2 h-2 rounded-full bg-ink" /> fact
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block w-2 h-2 rounded-full bg-accent" />{' '}
          pinned
        </span>
      </div>

      {selected ? (
        <div className="absolute top-2 right-2 w-72 border border-rule bg-bg p-3 flex flex-col gap-2">
          <div className="flex items-start justify-between gap-2">
            <p className="font-mono text-[12px] text-ink leading-snug">
              {selected.text}
            </p>
            <Button
              variant="icon"
              size="icon"
              onClick={() => setSelectedId(null)}
              aria-label="close fact card"
            >
              <X className="w-3.5 h-3.5" aria-hidden />
            </Button>
          </div>
          <div className="flex items-center gap-1.5 flex-wrap">
            {selected.entities.map((entity) => (
              <Badge key={entity}>{entity}</Badge>
            ))}
            <span className="font-mono text-[10px] lowercase text-ink-ghost">
              {selected.confidence}
              {selected.corroboration > 0 &&
                ` · seen ×${selected.corroboration + 1}`}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onPin(selected)}
              disabled={busy}
              className="gap-1"
            >
              {selected.pinned ? (
                <>
                  <PinOff className="w-3 h-3" aria-hidden /> unpin
                </>
              ) : (
                <>
                  <Pin className="w-3 h-3" aria-hidden /> pin
                </>
              )}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                onDelete(selected)
                setSelectedId(null)
              }}
              disabled={busy}
              className="gap-1"
            >
              <Trash2 className="w-3 h-3" aria-hidden /> delete
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  )
}
