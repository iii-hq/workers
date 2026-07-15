import { Pin, PinOff, Trash2, X } from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Input } from '@/components/ui/Input'
import type { MemoryFact } from '@/lib/memory'
import { cn } from '@/lib/utils'

/**
 * The bank as a schematic constellation, built to stay readable at 10k+
 * facts via level-of-detail:
 *
 * - Default view draws ENTITY HUBS ONLY, sized by fact count, capped to
 *   the top `MAX_HUBS` by degree (a banner names what's hidden; the
 *   search box refocuses on anything).
 * - Clicking a hub expands its facts as spokes, capped to `MAX_SPOKES`
 *   (pinned first, then newest) with a "+N more" node linking to the
 *   facts tab. Small banks (<= AUTO_EXPAND_FACTS facts) auto-expand.
 * - Wheel zooms around the cursor, drag pans, Fit resets. Pure SVG
 *   viewBox math, no graph library.
 *
 * Layout is deterministic (golden-angle spiral over hubs sorted by
 * degree), so the same bank always draws the same map. This is a
 * render-time projection of the flat store: `entities[]` IS the edge
 * list; storage stays flat.
 */

interface MemoryGraphProps {
  facts: MemoryFact[]
  totalFacts: number
  onPin: (fact: MemoryFact) => void
  onDelete: (fact: MemoryFact) => void
  onShowFacts: () => void
  busy: boolean
}

const MAX_HUBS = 40
const MAX_SPOKES = 20
const AUTO_EXPAND_FACTS = 30
const GOLDEN_ANGLE = 2.399963
const NO_ENTITY_HUB = '(no entities)'

interface HubNode {
  entity: string
  x: number
  y: number
  count: number
  r: number
  expanded: boolean
}

interface FactNode {
  fact: MemoryFact
  hub: string
  x: number
  y: number
}

interface MoreNode {
  hub: string
  x: number
  y: number
  hidden: number
}

interface Edge {
  x1: number
  y1: number
  x2: number
  y2: number
  factId: string
}

interface Layout {
  hubs: HubNode[]
  nodes: FactNode[]
  more: MoreNode[]
  edges: Edge[]
  hiddenHubs: number
  bounds: { x: number; y: number; w: number; h: number }
}

function hubRadius(count: number): number {
  return Math.min(22, 6 + 2.4 * Math.sqrt(count))
}

function spokeOrder(a: MemoryFact, b: MemoryFact): number {
  if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
  return b.updated_at - a.updated_at
}

function layoutGraph(
  facts: MemoryFact[],
  filter: string,
  expandedHubs: ReadonlySet<string>,
  autoExpand: boolean,
): Layout {
  const byEntity = new Map<string, MemoryFact[]>()
  for (const fact of facts) {
    const keys = fact.entities.length > 0 ? fact.entities : [NO_ENTITY_HUB]
    for (const entity of keys) {
      const list = byEntity.get(entity) ?? []
      list.push(fact)
      byEntity.set(entity, list)
    }
  }

  const needle = filter.trim().toLowerCase()
  let hubEntries = [...byEntity.entries()].sort(
    (a, b) => b[1].length - a[1].length || a[0].localeCompare(b[0]),
  )
  if (needle) {
    hubEntries = hubEntries.filter(([entity]) =>
      entity.toLowerCase().includes(needle),
    )
  }
  const hiddenHubs = Math.max(0, hubEntries.length - MAX_HUBS)
  hubEntries = hubEntries.slice(0, MAX_HUBS)

  // Spiral spacing scales with how much room each hub needs when
  // expanded, so clusters cannot sit inside each other.
  const hubs: HubNode[] = []
  const hubPos = new Map<string, HubNode>()
  hubEntries.forEach(([entity, list], i) => {
    const expanded = autoExpand || expandedHubs.has(entity)
    const step = expanded ? 150 : 95
    const r = step * Math.sqrt(i + (i === 0 ? 0 : 1.4))
    const theta = i * GOLDEN_ANGLE
    const node: HubNode = {
      entity,
      x: Math.round(r * Math.cos(theta)),
      y: Math.round(r * Math.sin(theta)),
      count: list.length,
      r: hubRadius(list.length),
      expanded,
    }
    hubs.push(node)
    hubPos.set(entity, node)
  })

  const nodes: FactNode[] = []
  const more: MoreNode[] = []
  const edges: Edge[] = []
  for (const hub of hubs) {
    if (!hub.expanded) continue
    const list = [...(byEntity.get(hub.entity) ?? [])].sort(spokeOrder)
    const shown = list.slice(0, MAX_SPOKES)
    const hidden = list.length - shown.length
    const slots = shown.length + (hidden > 0 ? 1 : 0)
    shown.forEach((fact, j) => {
      const ring = hub.r + 34 + 12 * Math.floor(j / 12)
      // Start angles at the upper-right and skip the straight-down slot
      // band where the hub label sits.
      const angle =
        -0.6 + (j % 12) * ((2 * Math.PI - 0.7) / Math.min(slots, 12))
      const x = hub.x + Math.round(ring * Math.cos(angle))
      const y = hub.y + Math.round(ring * Math.sin(angle))
      nodes.push({ fact, hub: hub.entity, x, y })
      edges.push({ x1: hub.x, y1: hub.y, x2: x, y2: y, factId: fact.id })
    })
    if (hidden > 0) {
      const ring = hub.r + 34
      const x = hub.x + Math.round(ring * Math.cos(-0.6 - 0.5))
      const y = hub.y + Math.round(ring * Math.sin(-0.6 - 0.5))
      more.push({ hub: hub.entity, x, y, hidden })
    }
  }

  const xs = [...hubs.map((h) => h.x), ...nodes.map((n) => n.x), 0]
  const ys = [...hubs.map((h) => h.y), ...nodes.map((n) => n.y), 0]
  const pad = 110
  const minX = Math.min(...xs) - pad
  const minY = Math.min(...ys) - pad
  return {
    hubs,
    nodes,
    more,
    edges,
    hiddenHubs,
    bounds: {
      x: minX,
      y: minY,
      w: Math.max(...xs) + pad - minX,
      h: Math.max(...ys) + pad - minY,
    },
  }
}

export function MemoryGraph({
  facts,
  totalFacts,
  onPin,
  onDelete,
  onShowFacts,
  busy,
}: MemoryGraphProps) {
  const live = useMemo(() => facts.filter((f) => f.invalid_at == null), [facts])
  const [filter, setFilter] = useState('')
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [hoverId, setHoverId] = useState<string | null>(null)
  const autoExpand = live.length <= AUTO_EXPAND_FACTS && !filter

  const layout = useMemo(
    () => layoutGraph(live, filter, expanded, autoExpand),
    [live, filter, expanded, autoExpand],
  )

  // Zoom/pan as a controlled viewBox; null = fit to layout bounds.
  const [view, setView] = useState<{
    x: number
    y: number
    w: number
    h: number
  } | null>(null)
  const svgRef = useRef<SVGSVGElement | null>(null)
  const drag = useRef<{ x: number; y: number } | null>(null)
  const vb = view ?? layout.bounds

  const clientToWorld = (cx: number, cy: number) => {
    const rect = svgRef.current?.getBoundingClientRect()
    if (!rect) return { x: 0, y: 0 }
    return {
      x: vb.x + ((cx - rect.left) / rect.width) * vb.w,
      y: vb.y + ((cy - rect.top) / rect.height) * vb.h,
    }
  }

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
    <div className="flex flex-col gap-2 flex-1 min-h-0">
      <div className="flex items-center gap-2 flex-wrap">
        <Input
          value={filter}
          onChange={setFilter}
          placeholder="focus entities..."
          aria-label="filter entities"
          className="w-56"
        />
        <span className="font-mono text-[11px] lowercase text-ink-faint">
          {layout.hubs.length} entities
          {layout.hiddenHubs > 0 &&
            ` (top ${MAX_HUBS} shown, ${layout.hiddenHubs} more — search to focus)`}
          {totalFacts > live.length &&
            ` · mapping newest ${live.length} of ${totalFacts} facts`}
        </span>
        <span className="flex-1" />
        <Button variant="ghost" size="sm" onClick={() => setView(null)}>
          fit
        </Button>
        {!autoExpand && expanded.size > 0 ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setExpanded(new Set())}
          >
            collapse all
          </Button>
        ) : null}
      </div>

      <div className="relative flex-1 min-h-0 border border-rule">
        <svg
          ref={svgRef}
          viewBox={`${vb.x} ${vb.y} ${vb.w} ${vb.h}`}
          className="w-full h-full min-h-[440px] cursor-grab active:cursor-grabbing select-none"
          role="img"
          aria-label="memory graph: entity hubs and fact nodes"
          onWheel={(e) => {
            const factor = e.deltaY > 0 ? 1.15 : 1 / 1.15
            const at = clientToWorld(e.clientX, e.clientY)
            const w = Math.min(
              Math.max(vb.w * factor, 120),
              layout.bounds.w * 4,
            )
            const h = (w / vb.w) * vb.h
            setView({
              x: at.x - ((at.x - vb.x) / vb.w) * w,
              y: at.y - ((at.y - vb.y) / vb.h) * h,
              w,
              h,
            })
          }}
          onPointerDown={(e) => {
            drag.current = { x: e.clientX, y: e.clientY }
            ;(e.target as Element).setPointerCapture?.(e.pointerId)
          }}
          onPointerMove={(e) => {
            if (!drag.current) return
            const rect = svgRef.current?.getBoundingClientRect()
            if (!rect) return
            const dx = ((e.clientX - drag.current.x) / rect.width) * vb.w
            const dy = ((e.clientY - drag.current.y) / rect.height) * vb.h
            drag.current = { x: e.clientX, y: e.clientY }
            setView({ x: vb.x - dx, y: vb.y - dy, w: vb.w, h: vb.h })
          }}
          onPointerUp={() => {
            drag.current = null
          }}
        >
          {layout.edges.map((edge) => (
            <line
              key={`${edge.factId}:${edge.x2}:${edge.y2}`}
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
            // biome-ignore lint/a11y/useSemanticElements: SVG nodes act as buttons
            <g
              key={hub.entity}
              role="button"
              tabIndex={0}
              aria-label={`entity ${hub.entity}: ${hub.count} facts`}
              className="cursor-pointer focus:outline-none"
              onClick={() => {
                if (autoExpand) return
                setExpanded((cur) => {
                  const next = new Set(cur)
                  if (next.has(hub.entity)) next.delete(hub.entity)
                  else next.add(hub.entity)
                  return next
                })
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') e.preventDefault()
              }}
            >
              <rect
                x={hub.x - hub.r}
                y={hub.y - hub.r}
                width={hub.r * 2}
                height={hub.r * 2}
                className={cn(
                  'fill-panel stroke-ink-faint',
                  hub.expanded && 'stroke-accent',
                )}
                strokeWidth={1.5}
              />
              <text
                x={hub.x}
                y={hub.y + 4}
                textAnchor="middle"
                className="fill-ink font-mono tabular-nums pointer-events-none"
                fontSize={11}
              >
                {hub.count}
              </text>
              <text
                x={hub.x}
                y={hub.y - hub.r - 8}
                textAnchor="middle"
                fontSize={11}
                className="fill-ink-faint font-mono lowercase pointer-events-none"
                paintOrder="stroke"
                stroke="var(--color-bg, #111110)"
                strokeWidth={4}
              >
                {hub.entity.length > 24
                  ? `${hub.entity.slice(0, 22)}…`
                  : hub.entity}
              </text>
            </g>
          ))}

          {layout.nodes.map(({ fact, x, y }) => (
            // biome-ignore lint/a11y/useSemanticElements: SVG nodes act as buttons
            <g
              key={`${fact.id}:${x}`}
              role="button"
              tabIndex={0}
              aria-label={`fact: ${fact.text.slice(0, 60)}`}
              className="cursor-pointer focus:outline-none"
              onClick={(e) => {
                e.stopPropagation()
                setSelectedId((cur) => (cur === fact.id ? null : fact.id))
              }}
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

          {layout.more.map((m) => (
            // biome-ignore lint/a11y/useSemanticElements: SVG nodes act as buttons
            <g
              key={`more:${m.hub}`}
              role="button"
              tabIndex={0}
              aria-label={`${m.hidden} more facts for ${m.hub} — open the facts tab`}
              className="cursor-pointer focus:outline-none"
              onClick={onShowFacts}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault()
                  onShowFacts()
                }
              }}
            >
              <text
                x={m.x}
                y={m.y}
                textAnchor="middle"
                fontSize={11}
                className="fill-accent font-mono lowercase"
                paintOrder="stroke"
                stroke="var(--color-bg, #111110)"
                strokeWidth={4}
              >
                +{m.hidden} more
              </text>
            </g>
          ))}
        </svg>

        <div className="absolute bottom-2 left-2 flex items-center gap-3 font-mono text-[10px] lowercase text-ink-ghost bg-bg/90 border border-rule-2 px-2 py-1">
          <span className="flex items-center gap-1">
            <span className="inline-block w-2 h-2 border border-ink-faint bg-panel" />{' '}
            entity (click to expand)
          </span>
          <span className="flex items-center gap-1">
            <span className="inline-block w-2 h-2 rounded-full bg-ink" /> fact
          </span>
          <span className="flex items-center gap-1">
            <span className="inline-block w-2 h-2 rounded-full bg-accent" />{' '}
            pinned
          </span>
          <span>wheel: zoom · drag: pan</span>
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
    </div>
  )
}
