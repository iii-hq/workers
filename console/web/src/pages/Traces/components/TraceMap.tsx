import '@xyflow/react/dist/style.css'

import {
  Background,
  BackgroundVariant,
  Controls,
  type Edge,
  type EdgeProps,
  getSimpleBezierPath,
  Handle,
  type Node,
  type NodeProps,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from '@xyflow/react'
import { useCallback, useEffect, useMemo } from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import { formatDuration } from '../lib/traceUtils'

interface TraceMapProps {
  data: WaterfallData
  onSpanClick: (span: VisualizationSpan) => void
  selectedSpanId?: string
}

interface WorkerStats {
  name: string
  spanCount: number
  totalDuration: number
  errorCount: number
}

interface WorkerNodeData extends Record<string, unknown> {
  stats: WorkerStats
  isSelected: boolean
}

interface WorkerEdgeData extends Record<string, unknown> {
  callCount: number
  totalDuration: number
  hasError: boolean
}

const NODE_WIDTH = 220

function WorkerNode({ data }: NodeProps<Node<WorkerNodeData>>) {
  const { stats, isSelected } = data
  const hasError = stats.errorCount > 0
  const tone = hasError ? 'alert' : 'accent'

  return (
    <div
      className={cn(
        'min-w-[200px] max-w-[260px] border border-rule bg-bg',
        isSelected && 'border-l-2 border-l-accent',
        hasError && !isSelected && 'border-l-2 border-l-alert',
      )}
      style={{ width: NODE_WIDTH }}
    >
      <Handle
        type="target"
        position={Position.Top}
        className="!bg-ink !border-bg !w-1.5 !h-1.5 !min-w-0 !min-h-0"
      />
      <div
        className={cn(
          'border-b border-rule-2 px-2 py-1 flex items-center gap-2',
          hasError ? 'bg-bg' : 'bg-panel',
        )}
      >
        <StatusDot tone={tone} />
        <span
          className={cn(
            'font-mono text-[11px] uppercase tracking-[0.06em] truncate',
            hasError ? 'text-alert' : 'text-ink-faint',
          )}
        >
          {stats.name}
        </span>
      </div>
      <div className="px-2 py-1.5 flex items-center gap-2 font-mono text-[11px] text-ink-faint tabular-nums">
        <span className="lowercase truncate">
          {stats.spanCount} span{stats.spanCount !== 1 ? 's' : ''}
        </span>
        <span className="ml-auto">{formatDuration(stats.totalDuration)}</span>
      </div>
      {hasError && (
        <div className="px-2 pb-1 font-mono text-[11px] text-alert tabular-nums lowercase">
          {stats.errorCount} err
        </div>
      )}
      <Handle
        type="source"
        position={Position.Bottom}
        className="!bg-ink !border-bg !w-1.5 !h-1.5 !min-w-0 !min-h-0"
      />
    </div>
  )
}

function WorkerEdge({
  sourceX,
  sourceY,
  targetX,
  targetY,
  data,
  markerEnd,
}: EdgeProps<Edge<WorkerEdgeData>>) {
  const [edgePath, labelX, labelY] = getSimpleBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
  })
  const hasError = data?.hasError ?? false
  const stroke = hasError ? 'var(--color-alert)' : 'var(--color-ink)'

  return (
    <>
      <path
        d={edgePath}
        fill="none"
        stroke={stroke}
        strokeWidth={1}
        markerEnd={markerEnd}
      />
      {data && (
        <foreignObject
          x={labelX - 60}
          y={labelY - 14}
          width={120}
          height={28}
          className="pointer-events-none"
        >
          <div className="flex items-center justify-center">
            <span
              className={cn(
                'inline-flex items-center gap-1 border border-rule bg-bg px-1.5 py-0.5 font-mono text-[10px] tabular-nums lowercase',
                hasError ? 'text-alert' : 'text-ink-faint',
              )}
            >
              {data.callCount} call{data.callCount !== 1 ? 's' : ''}
            </span>
          </div>
        </foreignObject>
      )}
    </>
  )
}

const nodeTypes = { worker: WorkerNode }
const edgeTypes = { worker: WorkerEdge }

interface WorkerGraph {
  nodes: Node<WorkerNodeData>[]
  edges: Edge<WorkerEdgeData>[]
}

function buildWorkerGraph(
  data: WaterfallData,
  selectedSpanId: string | undefined,
): WorkerGraph {
  const workerMap = new Map<string, WorkerStats>()
  const edgeMap = new Map<
    string,
    {
      from: string
      to: string
      callCount: number
      totalDuration: number
      hasError: boolean
    }
  >()

  for (const span of data.spans) {
    const worker = span.service_name || span.name.split('.')[0] || 'unknown'
    const existing = workerMap.get(worker) ?? {
      name: worker,
      spanCount: 0,
      totalDuration: 0,
      errorCount: 0,
    }
    workerMap.set(worker, {
      name: worker,
      spanCount: existing.spanCount + 1,
      totalDuration: existing.totalDuration + (span.duration_ms || 0),
      errorCount: existing.errorCount + (span.status === 'error' ? 1 : 0),
    })
  }

  const spanById = new Map(data.spans.map((s) => [s.span_id, s]))

  for (const span of data.spans) {
    if (!span.parent_span_id) continue
    const parent = spanById.get(span.parent_span_id)
    if (!parent) continue
    const from = parent.service_name || parent.name.split('.')[0] || 'unknown'
    const to = span.service_name || span.name.split('.')[0] || 'unknown'
    if (from === to) continue
    const key = `${from}->${to}`
    const existing = edgeMap.get(key) ?? {
      from,
      to,
      callCount: 0,
      totalDuration: 0,
      hasError: false,
    }
    edgeMap.set(key, {
      from,
      to,
      callCount: existing.callCount + 1,
      totalDuration: existing.totalDuration + (span.duration_ms || 0),
      hasError: existing.hasError || span.status === 'error',
    })
  }

  const workerNames = Array.from(workerMap.keys())
  const selectedWorker = selectedSpanId
    ? (() => {
        const span = spanById.get(selectedSpanId)
        if (!span) return undefined
        return span.service_name || span.name.split('.')[0] || 'unknown'
      })()
    : undefined

  // Radial layout — stable, deterministic, schematic-friendly.
  const nodeCount = Math.max(1, workerNames.length)
  const centerX = 0
  const centerY = 0
  const radius = nodeCount === 1 ? 0 : Math.max(180, 90 + nodeCount * 36)

  const nodes: Node<WorkerNodeData>[] = workerNames.map((name, index) => {
    const stats = workerMap.get(name)
    if (!stats) {
      return {
        id: name,
        type: 'worker',
        position: { x: 0, y: 0 },
        data: {
          stats: {
            name,
            spanCount: 0,
            totalDuration: 0,
            errorCount: 0,
          },
          isSelected: false,
        },
      }
    }
    const angle = (index / nodeCount) * 2 * Math.PI - Math.PI / 2
    return {
      id: name,
      type: 'worker',
      position: {
        x: centerX + radius * Math.cos(angle) - NODE_WIDTH / 2,
        y: centerY + radius * Math.sin(angle),
      },
      data: {
        stats,
        isSelected: selectedWorker === name,
      },
    }
  })

  const edges: Edge<WorkerEdgeData>[] = Array.from(edgeMap.values()).map(
    (edge) => ({
      id: `${edge.from}->${edge.to}`,
      source: edge.from,
      target: edge.to,
      type: 'worker',
      data: {
        callCount: edge.callCount,
        totalDuration: edge.totalDuration,
        hasError: edge.hasError,
      },
    }),
  )

  return { nodes, edges }
}

function TraceMapInner({ data, onSpanClick, selectedSpanId }: TraceMapProps) {
  const { fitView } = useReactFlow()

  const { nodes, edges } = useMemo(
    () => buildWorkerGraph(data, selectedSpanId),
    [data, selectedSpanId],
  )

  const handleNodeClick = useCallback(
    (_event: React.MouseEvent, node: Node) => {
      const span = data.spans.find((s) => {
        const worker = s.service_name || s.name.split('.')[0] || 'unknown'
        return worker === node.id
      })
      if (span) onSpanClick(span)
    },
    [data.spans, onSpanClick],
  )

  const workerCount = nodes.length
  const connectionCount = edges.length

  // biome-ignore lint/correctness/useExhaustiveDependencies: refit when graph shape changes
  useEffect(() => {
    // Defer to next frame to ensure layout is measured.
    const id = requestAnimationFrame(() => {
      fitView({ padding: 0.2, duration: 0 })
    })
    return () => cancelAnimationFrame(id)
  }, [fitView, workerCount, connectionCount])

  return (
    <div className="flex flex-col h-full bg-bg">
      <div className="flex items-center justify-between px-4 py-2 border-b border-rule bg-panel flex-shrink-0">
        <div className="flex items-center gap-3 font-mono text-[11px] text-ink-faint tabular-nums lowercase">
          <span>
            {workerCount} worker{workerCount !== 1 ? 's' : ''}
          </span>
          <span className="text-ink-ghost">•</span>
          <span>
            {connectionCount} connection{connectionCount !== 1 ? 's' : ''}
          </span>
        </div>
        <span className="font-mono text-[11px] text-ink-faint lowercase">
          click a node to view worker details
        </span>
      </div>

      <div className="flex-1 min-h-0">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          onNodeClick={handleNodeClick}
          fitView
          fitViewOptions={{ padding: 0.2 }}
          minZoom={0.2}
          maxZoom={2}
          proOptions={{ hideAttribution: true }}
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable
          panOnDrag
          zoomOnScroll
        >
          <Background
            variant={BackgroundVariant.Cross}
            gap={9999}
            color="var(--color-rule)"
          />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>
    </div>
  )
}

export function TraceMap(props: TraceMapProps) {
  return (
    <ReactFlowProvider>
      <TraceMapInner {...props} />
    </ReactFlowProvider>
  )
}
