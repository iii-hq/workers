import {
  Background,
  BackgroundVariant,
  Controls,
  type Edge,
  type EdgeProps,
  getBezierPath,
  Handle,
  type Node,
  type NodeProps,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from '@xyflow/react'
import dagre from 'dagre'
import { useCallback, useEffect, useMemo } from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import {
  classifySpanType,
  formatDuration,
  getServiceName,
} from '../lib/traceUtils'

interface FlowViewProps {
  data: WaterfallData
  onSpanClick: (span: VisualizationSpan) => void
  selectedSpanId?: string
}

interface SpanNodeData extends Record<string, unknown> {
  span: VisualizationSpan
  isSelected: boolean
  operationName: string
  spanType: string
}

interface SpanEdgeData extends Record<string, unknown> {
  hasError: boolean
}

const NODE_WIDTH = 220
const NODE_HEIGHT = 64

function SpanNode({ data }: NodeProps<Node<SpanNodeData>>) {
  const { span, isSelected, operationName, spanType } = data
  const hasError = span.status === 'error'
  const tone = hasError ? 'alert' : 'accent'

  return (
    <div
      className={cn(
        'min-w-[180px] max-w-[280px] border border-rule bg-bg',
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
          title={operationName}
        >
          {operationName}
        </span>
        <span className="ml-auto font-mono text-[10px] uppercase tracking-[0.06em] text-ink-ghost shrink-0">
          {spanType}
        </span>
      </div>
      <div className="px-2 py-1.5 flex items-center gap-2 font-mono text-[11px] text-ink-faint tabular-nums">
        <span className="lowercase truncate">{getServiceName(span)}</span>
        <span className="ml-auto">{formatDuration(span.duration_ms)}</span>
      </div>
      <Handle
        type="source"
        position={Position.Bottom}
        className="!bg-ink !border-bg !w-1.5 !h-1.5 !min-w-0 !min-h-0"
      />
    </div>
  )
}

function SpanEdge({
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  markerEnd,
}: EdgeProps<Edge<SpanEdgeData>>) {
  const [edgePath] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  })
  const hasError = data?.hasError ?? false
  const stroke = hasError ? 'var(--color-alert)' : 'var(--color-ink)'

  return (
    <path
      d={edgePath}
      fill="none"
      stroke={stroke}
      strokeWidth={1}
      markerEnd={markerEnd}
    />
  )
}

const nodeTypes = { span: SpanNode }
const edgeTypes = { span: SpanEdge }

interface FlowGraph {
  nodes: Node<SpanNodeData>[]
  edges: Edge<SpanEdgeData>[]
}

function buildFlowGraph(
  data: WaterfallData,
  selectedSpanId: string | undefined,
): FlowGraph {
  if (data.spans.length === 0) {
    return { nodes: [], edges: [] }
  }

  const graph = new dagre.graphlib.Graph()
  graph.setGraph({
    rankdir: 'TB',
    nodesep: 32,
    ranksep: 60,
    marginx: 24,
    marginy: 24,
  })
  graph.setDefaultEdgeLabel(() => ({}))

  const spanById = new Map(data.spans.map((s) => [s.span_id, s]))

  for (const span of data.spans) {
    graph.setNode(span.span_id, { width: NODE_WIDTH, height: NODE_HEIGHT })
  }

  for (const span of data.spans) {
    if (span.parent_span_id && spanById.has(span.parent_span_id)) {
      graph.setEdge(span.parent_span_id, span.span_id)
    }
  }

  dagre.layout(graph)

  const nodes: Node<SpanNodeData>[] = data.spans.map((span) => {
    const layoutNode = graph.node(span.span_id)
    const operationName = span.name
    const spanType = classifySpanType(span)
    return {
      id: span.span_id,
      type: 'span',
      position: {
        x: (layoutNode?.x ?? 0) - NODE_WIDTH / 2,
        y: (layoutNode?.y ?? 0) - NODE_HEIGHT / 2,
      },
      data: {
        span,
        isSelected: selectedSpanId === span.span_id,
        operationName,
        spanType,
      },
    }
  })

  const edges: Edge<SpanEdgeData>[] = []
  for (const span of data.spans) {
    if (!span.parent_span_id || !spanById.has(span.parent_span_id)) continue
    edges.push({
      id: `${span.parent_span_id}->${span.span_id}`,
      source: span.parent_span_id,
      target: span.span_id,
      type: 'span',
      data: { hasError: span.status === 'error' },
    })
  }

  return { nodes, edges }
}

function FlowViewInner({ data, onSpanClick, selectedSpanId }: FlowViewProps) {
  const { fitView } = useReactFlow()

  const { nodes, edges } = useMemo(
    () => buildFlowGraph(data, selectedSpanId),
    [data, selectedSpanId],
  )

  const handleNodeClick = useCallback(
    (_event: React.MouseEvent, node: Node) => {
      const span = data.spans.find((s) => s.span_id === node.id)
      if (span) onSpanClick(span)
    },
    [data.spans, onSpanClick],
  )

  const spanCount = nodes.length
  const edgeCount = edges.length

  // biome-ignore lint/correctness/useExhaustiveDependencies: refit when graph shape changes
  useEffect(() => {
    const id = requestAnimationFrame(() => {
      fitView({ padding: 0.15, duration: 0 })
    })
    return () => cancelAnimationFrame(id)
  }, [fitView, spanCount, edgeCount])

  return (
    <div className="flex flex-col h-full bg-bg">
      <div className="flex items-center justify-between px-4 py-2 border-b border-rule bg-panel flex-shrink-0">
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          flow
        </span>
        <span className="font-mono text-[11px] text-ink-faint tabular-nums lowercase">
          {spanCount} span{spanCount !== 1 ? 's' : ''}
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
          fitViewOptions={{ padding: 0.15 }}
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

export function FlowView(props: FlowViewProps) {
  return (
    <ReactFlowProvider>
      <FlowViewInner {...props} />
    </ReactFlowProvider>
  )
}
