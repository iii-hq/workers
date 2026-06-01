/**
 * Flame-graph view: spans rendered to a <canvas> for performance.
 *
 * Schematic theming: the flame rectangles render to a `<canvas>`, so we
 * cannot use Tailwind classes directly. Instead we resolve the schematic
 * tokens (`--color-ink`, `--color-alert`, `--color-warn`, `--color-accent`,
 * `--color-bg`, `--color-panel`, `--color-rule`, `--color-rule-2`,
 * `--color-ink-faint`, `--color-ink-ghost`) from
 * `getComputedStyle(document.documentElement)` each frame. The host theme
 * (`[data-theme="light"]` / dark) flips automatically because the same
 * variable names re-resolve.
 *
 * Depth tiling: per DESIGN.md §3 the schematic is monochrome. Service
 * identity is encoded through ink shades by depth, never through a
 * chromatic palette. Errors collapse onto `--color-alert`, pending/unset
 * onto `--color-warn`, and the selected rectangle gets a 2px accent
 * outline (the single accent moment per region).
 *
 * Surrounding chrome (toolbar, legend, tooltip, keyboard hint) uses the
 * project's Tailwind primitives so theme swaps re-skin those areas
 * without canvas redraws.
 */

import { Eye, Minus, Plus, RotateCcw, Sparkles } from 'lucide-react'
import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { cn } from '@/lib/utils'
import { useShowEngineRouting } from '../hooks/useShowEngineRouting'
import { buildSpanTree, flattenTree } from '../lib/spanTree'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import { formatDuration } from '../lib/traceUtils'
import { flattenPreorder } from '../lib/treeFlatten'
import { IconToggleButton } from './IconToggleButton'

interface FlameGraphProps {
  data: WaterfallData
  onSpanClick: (span: VisualizationSpan) => void
  selectedSpanId?: string
}

interface FlameNode {
  span: VisualizationSpan
  x: number
  width: number
  depth: number
  children: FlameNode[]
  selfTime: number
}

interface ResolvedTokens {
  bg: string
  panel: string
  rule: string
  rule2: string
  ink: string
  inkFaint: string
  inkGhost: string
  accent: string
  alert: string
  warn: string
}

interface ViewState {
  hoveredNode: FlameNode | null
  tooltipPos: { x: number; y: number }
  zoomLevel: number
  panOffset: number
}

type ViewAction =
  | { type: 'SET_HOVERED'; node: FlameNode | null; x: number; y: number }
  | { type: 'SET_ZOOM'; zoomLevel: number }
  | { type: 'SET_PAN'; panOffset: number }
  | { type: 'RESET_ZOOM' }

const initialViewState: ViewState = {
  hoveredNode: null,
  tooltipPos: { x: 0, y: 0 },
  zoomLevel: 1,
  panOffset: 0,
}

function viewReducer(state: ViewState, action: ViewAction): ViewState {
  switch (action.type) {
    case 'SET_HOVERED':
      return {
        ...state,
        hoveredNode: action.node,
        tooltipPos: { x: action.x, y: action.y },
      }
    case 'SET_ZOOM':
      return { ...state, zoomLevel: action.zoomLevel }
    case 'SET_PAN':
      return { ...state, panOffset: action.panOffset }
    case 'RESET_ZOOM':
      return { ...state, zoomLevel: 1, panOffset: 0 }
  }
}

function buildFlameNodes(spans: VisualizationSpan[]): FlameNode[] {
  const spanMap = new Map<string, FlameNode>()
  const roots: FlameNode[] = []

  for (const span of spans) {
    const node: FlameNode = {
      span,
      x: span.start_percent,
      width: span.width_percent,
      depth: span.depth,
      children: [],
      selfTime: span.duration_ms,
    }
    spanMap.set(span.span_id, node)
  }

  for (const span of spans) {
    const node = spanMap.get(span.span_id)
    if (!node) continue

    if (span.parent_span_id && spanMap.has(span.parent_span_id)) {
      const parent = spanMap.get(span.parent_span_id)
      if (parent) parent.children.push(node)
    } else {
      roots.push(node)
    }
  }

  // self-time = duration minus children's duration
  for (const node of spanMap.values()) {
    let childrenDuration = 0
    for (const child of node.children) {
      childrenDuration += child.span.duration_ms
    }
    node.selfTime = Math.max(0, node.span.duration_ms - childrenDuration)
  }

  return roots
}

function flattenFlameNodes(nodes: FlameNode[]): FlameNode[] {
  // Iterative (not recursive): deep flame trees (thousands of nested spans)
  // would overflow the call stack — the same hazard the span/waterfall
  // transforms were rewritten to avoid.
  return flattenPreorder(nodes)
}

/**
 * Resolve the schematic tokens at draw time. We pull them off
 * `document.documentElement` so a `[data-theme="light"]` swap is picked
 * up on the next redraw without re-mounting the canvas.
 */
function resolveTokens(): ResolvedTokens {
  const root = getComputedStyle(document.documentElement)
  const get = (name: string) => root.getPropertyValue(name).trim()
  return {
    bg: get('--color-bg'),
    panel: get('--color-panel'),
    rule: get('--color-rule'),
    rule2: get('--color-rule-2'),
    ink: get('--color-ink'),
    inkFaint: get('--color-ink-faint'),
    inkGhost: get('--color-ink-ghost'),
    accent: get('--color-accent'),
    alert: get('--color-alert'),
    warn: get('--color-warn'),
  }
}

/**
 * Monochrome ink shades tiled by depth. Errors and pending spans
 * collapse onto `alert` / `warn`. Cycles through 4 shades for any depth
 * beyond 3.
 */
function getDepthFill(node: FlameNode, tokens: ResolvedTokens): string {
  if (node.span.status === 'error') return tokens.alert
  if (node.span.status === 'unset') return tokens.warn
  // depth 0 = full ink, depth 1 = faint, depth 2 = ghost, depth 3+ = rule
  const cycle = node.depth % 4
  if (cycle === 0) return tokens.ink
  if (cycle === 1) return tokens.inkFaint
  if (cycle === 2) return tokens.inkGhost
  return tokens.rule
}

/**
 * Text color inside a rectangle. Ink-tinted bars use paper (`bg`) for
 * legibility on dark fills (and ink-on-bg fills in light mode). For
 * alert/warn fills we also use `bg` (paper) so the high-contrast text
 * works in both themes.
 */
function getRectTextColor(_node: FlameNode, tokens: ResolvedTokens): string {
  return tokens.bg
}

export function FlameGraph({
  data,
  onSpanClick,
  selectedSpanId,
}: FlameGraphProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const minimapRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const [viewState, dispatch] = useReducer(viewReducer, initialViewState)
  const { hoveredNode, tooltipPos, zoomLevel, panOffset } = viewState

  // Same defaults as the waterfall view: critical-path-only is on, engine
  // routing is hidden. The engine-routing toggle is persisted via the
  // shared hook so the preference survives view swaps.
  const [showCriticalPath, setShowCriticalPath] = useState(true)
  const [showEngineRouting, setShowEngineRouting] = useShowEngineRouting()

  const ROW_HEIGHT = 26
  const ROW_GAP = 2
  const PADDING = 16
  const MINIMAP_HEIGHT = 32

  // selfTime + child counts come from buildFlameNodes over the unfiltered
  // input so the tooltip reports accurate metrics even when a span's
  // children are not currently rendered.
  const allFlameNodes = useMemo(
    () => flattenFlameNodes(buildFlameNodes(data.spans)),
    [data.spans],
  )

  // The visible set is computed via the same tree pipeline the waterfall
  // uses, so both views agree on what "only critical path" and
  // "show engine routing" mean. `displayDepth` from `flattenTree`
  // overrides each FlameNode's depth so hidden routing ancestors shift
  // descendants left here as well.
  const flatNodes = useMemo(() => {
    const spanTree = buildSpanTree(data.spans)
    const allIds = new Set(data.spans.map((s) => s.span_id))
    const filteredRows = flattenTree(spanTree, {
      expandedIds: allIds,
      hideEngineRouting: !showEngineRouting,
      collapseEngineRoutingPairs: showEngineRouting,
      onlyCriticalPath: showCriticalPath,
    })
    const flameMap = new Map(allFlameNodes.map((n) => [n.span.span_id, n]))
    const result: FlameNode[] = []
    for (const row of filteredRows) {
      const flame = flameMap.get(row.span_id)
      if (!flame) continue
      result.push({ ...flame, depth: row.displayDepth })
    }
    return result
  }, [data.spans, allFlameNodes, showCriticalPath, showEngineRouting])

  const maxDepth = useMemo(
    // reduce, not `Math.max(0, ...spread)`: a wide trace with >~100k visible
    // nodes would overflow the argument limit and throw during render.
    () => flatNodes.reduce((m, n) => (n.depth > m ? n.depth : m), 0) + 1,
    [flatNodes],
  )

  const draw = useCallback(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return

    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const tokens = resolveTokens()
    const dpr = window.devicePixelRatio || 1
    const width = container.clientWidth
    const height = maxDepth * (ROW_HEIGHT + ROW_GAP) + PADDING * 2 + 20 // extra for time ruler

    canvas.width = width * dpr
    canvas.height = height * dpr
    canvas.style.width = `${width}px`
    canvas.style.height = `${height}px`
    ctx.scale(dpr, dpr)

    // clear with paper
    ctx.fillStyle = tokens.bg
    ctx.fillRect(0, 0, width, height)

    // subtle grid lines (rule-2 = quieter rule)
    ctx.strokeStyle = tokens.rule2
    ctx.lineWidth = 1
    for (let i = 0; i <= 4; i++) {
      const x = PADDING + (width - PADDING * 2) * (i / 4)
      ctx.beginPath()
      ctx.moveTo(x, PADDING)
      ctx.lineTo(x, height - PADDING - 16)
      ctx.stroke()
    }

    // draw spans
    const graphWidth = (width - PADDING * 2) * zoomLevel
    const offsetX = panOffset

    for (const node of flatNodes) {
      const x = PADDING + (node.x / 100) * graphWidth - offsetX
      const w = Math.max(2, (node.width / 100) * graphWidth)
      const y = PADDING + node.depth * (ROW_HEIGHT + ROW_GAP)

      // skip if out of view
      if (x + w < 0 || x > width) continue

      const isHovered = hoveredNode?.span.span_id === node.span.span_id
      const isSelected = selectedSpanId === node.span.span_id

      // depth-based alpha: bars further down get slightly faded so the
      // top of the call stack stays visually dominant. Selected and
      // hovered always render at full intensity.
      const depthAlpha = Math.max(0.7, 1 - node.depth * 0.05)

      const fillColor = getDepthFill(node, tokens)
      ctx.globalAlpha = isHovered || isSelected ? 1 : depthAlpha
      ctx.fillStyle = fillColor
      // rectilinear — no rounded corners
      ctx.fillRect(x, y, w, ROW_HEIGHT)

      // 1px separator border between adjacent rects (rule-2)
      ctx.globalAlpha = 1
      ctx.strokeStyle = tokens.rule2
      ctx.lineWidth = 1
      ctx.strokeRect(x + 0.5, y + 0.5, w - 1, ROW_HEIGHT - 1)

      // selected: 2px accent outline (the single accent moment)
      if (isSelected) {
        ctx.strokeStyle = tokens.accent
        ctx.lineWidth = 2
        ctx.strokeRect(x + 1, y + 1, w - 2, ROW_HEIGHT - 2)
      } else if (isHovered) {
        // hovered: 1px ink outline so the bar stands out without
        // claiming the accent slot
        ctx.strokeStyle = tokens.ink
        ctx.lineWidth = 1
        ctx.strokeRect(x + 0.5, y + 0.5, w - 1, ROW_HEIGHT - 1)
      }

      ctx.globalAlpha = 1

      // draw text if wide enough
      if (w > 50) {
        ctx.fillStyle = getRectTextColor(node, tokens)
        ctx.font = '11px "JetBrains Mono", monospace'
        ctx.textBaseline = 'middle'

        const spanName = node.span.name.toLowerCase()
        const duration = formatDuration(node.span.duration_ms)
        const maxTextWidth = w - 12

        // try to fit name + duration
        const fullText = `${spanName}  ${duration}`
        let displayText = fullText

        if (ctx.measureText(fullText).width > maxTextWidth) {
          displayText = spanName
          while (
            ctx.measureText(displayText).width > maxTextWidth &&
            displayText.length > 3
          ) {
            displayText = `${displayText.slice(0, -4)}...`
          }
        }

        if (displayText.length > 3) {
          ctx.fillText(displayText, x + 6, y + ROW_HEIGHT / 2)
        }
      } else if (w > 20) {
        // medium bars: just the duration
        ctx.fillStyle = getRectTextColor(node, tokens)
        ctx.globalAlpha = 0.85
        ctx.font = '10px "JetBrains Mono", monospace'
        ctx.textBaseline = 'middle'
        const dur = formatDuration(node.span.duration_ms)
        if (ctx.measureText(dur).width < w - 6) {
          ctx.fillText(dur, x + 3, y + ROW_HEIGHT / 2)
        }
        ctx.globalAlpha = 1
      }
    }

    // time ruler at bottom (ink-faint)
    ctx.fillStyle = tokens.inkFaint
    ctx.font = '10px "JetBrains Mono", monospace'
    ctx.textBaseline = 'top'
    for (let i = 0; i <= 4; i++) {
      const x = PADDING + (width - PADDING * 2) * (i / 4)
      const time = formatDuration(
        (data.total_duration_ms / zoomLevel) * (i / 4) +
          (panOffset / graphWidth) * data.total_duration_ms,
      )
      const textWidth = ctx.measureText(time).width
      const textX = i === 4 ? x - textWidth : i === 0 ? x : x - textWidth / 2
      ctx.fillText(time, textX, height - PADDING + 2)
    }
  }, [
    flatNodes,
    maxDepth,
    hoveredNode,
    selectedSpanId,
    zoomLevel,
    panOffset,
    data.total_duration_ms,
  ])

  // minimap (only renders when zoomed)
  const drawMinimap = useCallback(() => {
    const minimap = minimapRef.current
    const container = containerRef.current
    if (!minimap || !container || zoomLevel <= 1) return

    const ctx = minimap.getContext('2d')
    if (!ctx) return

    const tokens = resolveTokens()
    const dpr = window.devicePixelRatio || 1
    const width = container.clientWidth
    const height = MINIMAP_HEIGHT

    minimap.width = width * dpr
    minimap.height = height * dpr
    minimap.style.width = `${width}px`
    minimap.style.height = `${height}px`
    ctx.scale(dpr, dpr)

    ctx.fillStyle = tokens.panel
    ctx.fillRect(0, 0, width, height)

    // thin bars for every span, monochrome ink + status overrides
    const barHeight = Math.max(1, Math.min(3, height / (maxDepth + 1)))
    const topPad = (height - maxDepth * barHeight) / 2

    for (const node of flatNodes) {
      const x = PADDING + (node.x / 100) * (width - PADDING * 2)
      const w = Math.max(1, (node.width / 100) * (width - PADDING * 2))
      const y = topPad + node.depth * barHeight

      ctx.fillStyle = getDepthFill(node, tokens)
      ctx.globalAlpha = 0.6
      ctx.fillRect(x, y, w, Math.max(1, barHeight - 0.5))
    }

    ctx.globalAlpha = 1

    // viewport indicator — accent outline + faint accent wash
    const graphWidth = (width - PADDING * 2) * zoomLevel
    const viewStart = (panOffset / graphWidth) * (width - PADDING * 2)
    const viewWidth = (width - PADDING * 2) / zoomLevel

    ctx.strokeStyle = tokens.accent
    ctx.lineWidth = 1
    ctx.globalAlpha = 0.7
    ctx.strokeRect(PADDING + viewStart, 1, viewWidth, height - 2)

    ctx.fillStyle = tokens.accent
    ctx.globalAlpha = 0.08
    ctx.fillRect(PADDING + viewStart, 1, viewWidth, height - 2)

    ctx.globalAlpha = 1
  }, [flatNodes, maxDepth, zoomLevel, panOffset])

  useEffect(() => {
    draw()
    drawMinimap()

    const handleResize = () => {
      draw()
      drawMinimap()
    }
    window.addEventListener('resize', handleResize)
    return () => window.removeEventListener('resize', handleResize)
  }, [draw, drawMinimap])

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return

    const rect = canvas.getBoundingClientRect()
    const x = e.clientX - rect.left
    const y = e.clientY - rect.top
    const width = container.clientWidth
    const graphWidth = (width - PADDING * 2) * zoomLevel

    let found: FlameNode | null = null
    for (const node of flatNodes) {
      const nodeX = PADDING + (node.x / 100) * graphWidth - panOffset
      const nodeW = Math.max(2, (node.width / 100) * graphWidth)
      const nodeY = PADDING + node.depth * (ROW_HEIGHT + ROW_GAP)

      if (
        x >= nodeX &&
        x <= nodeX + nodeW &&
        y >= nodeY &&
        y <= nodeY + ROW_HEIGHT
      ) {
        found = node
        break
      }
    }

    // Only dispatch when the hover TARGET changes. A same-bar move (or
    // empty→empty over whitespace) would otherwise re-render and, since
    // `hoveredNode` drives the `draw` callback, force a full canvas redraw of
    // every bar on each mouse pixel — the freeze WaterfallChart removed for
    // the same reason. The tooltip therefore anchors at the bar's entry point.
    const foundId = found?.span.span_id ?? null
    const currentId = hoveredNode?.span.span_id ?? null
    if (foundId === currentId) return

    dispatch({ type: 'SET_HOVERED', node: found, x: e.clientX, y: e.clientY })
  }

  const handleMouseLeave = () => {
    dispatch({ type: 'SET_HOVERED', node: null, x: 0, y: 0 })
  }

  const handleClick = () => {
    if (hoveredNode) {
      onSpanClick(hoveredNode.span)
    }
  }

  const handleWheel = (e: React.WheelEvent<HTMLCanvasElement>) => {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault()
      const delta = e.deltaY > 0 ? 0.9 : 1.1
      dispatch({
        type: 'SET_ZOOM',
        zoomLevel: Math.max(1, Math.min(20, zoomLevel * delta)),
      })
    } else {
      dispatch({
        type: 'SET_PAN',
        panOffset: Math.max(0, panOffset + e.deltaY),
      })
    }
  }

  const handleMinimapClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const minimap = minimapRef.current
    const container = containerRef.current
    if (!minimap || !container) return

    const rect = minimap.getBoundingClientRect()
    const clickX = e.clientX - rect.left
    const width = container.clientWidth
    const graphWidth = (width - PADDING * 2) * zoomLevel

    const clickPercent = (clickX - PADDING) / (width - PADDING * 2)
    const newOffset = clickPercent * graphWidth - (width - PADDING * 2) / 2
    dispatch({ type: 'SET_PAN', panOffset: Math.max(0, newOffset) })
  }

  const handleReset = () => {
    dispatch({ type: 'RESET_ZOOM' })
  }

  const zoomIn = () =>
    dispatch({ type: 'SET_ZOOM', zoomLevel: Math.min(20, zoomLevel * 1.3) })
  const zoomOut = () =>
    dispatch({ type: 'SET_ZOOM', zoomLevel: Math.max(1, zoomLevel / 1.3) })

  const tracePercent = hoveredNode
    ? ((hoveredNode.span.duration_ms / data.total_duration_ms) * 100).toFixed(1)
    : '0'

  const hoveredStatusTone: 'accent' | 'alert' | 'warn' =
    hoveredNode?.span.status === 'error'
      ? 'alert'
      : hoveredNode?.span.status === 'ok'
        ? 'accent'
        : 'warn'

  return (
    <div className="flex flex-col h-full">
      {/* compact toolbar — view toggles + depth legend + zoom controls */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-rule bg-panel flex-shrink-0">
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1.5">
            <IconToggleButton
              active={showCriticalPath}
              onClick={() => setShowCriticalPath(!showCriticalPath)}
              label="only critical path"
            >
              <Sparkles className="w-3.5 h-3.5" />
            </IconToggleButton>
            <IconToggleButton
              active={showEngineRouting}
              onClick={() => setShowEngineRouting(!showEngineRouting)}
              label="show engine routing spans"
            >
              <Eye className="w-3.5 h-3.5" />
            </IconToggleButton>
          </div>

          <div aria-hidden className="w-px h-4 bg-rule-2 mx-1" />

          <span className="text-[10px] uppercase tracking-[0.06em] text-ink-ghost">
            depth
          </span>
          <div className="flex items-center gap-2">
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="flex items-center gap-1">
                  <span aria-hidden className="w-2 h-2 bg-ink" />
                  <span className="text-[10px] text-ink-faint lowercase tabular-nums">
                    0
                  </span>
                </div>
              </TooltipTrigger>
              <TooltipContent>root</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="flex items-center gap-1">
                  <span aria-hidden className="w-2 h-2 bg-ink-faint" />
                  <span className="text-[10px] text-ink-faint lowercase tabular-nums">
                    1
                  </span>
                </div>
              </TooltipTrigger>
              <TooltipContent>depth 1</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="flex items-center gap-1">
                  <span aria-hidden className="w-2 h-2 bg-ink-ghost" />
                  <span className="text-[10px] text-ink-faint lowercase tabular-nums">
                    2
                  </span>
                </div>
              </TooltipTrigger>
              <TooltipContent>depth 2</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="flex items-center gap-1">
                  <span aria-hidden className="w-2 h-2 bg-rule" />
                  <span className="text-[10px] text-ink-faint lowercase tabular-nums">
                    3+
                  </span>
                </div>
              </TooltipTrigger>
              <TooltipContent>depth 3 and deeper</TooltipContent>
            </Tooltip>
          </div>

          <div aria-hidden className="w-px h-3.5 bg-rule-2 mx-1.5" />

          <Tooltip>
            <TooltipTrigger asChild>
              <div className="flex items-center gap-1">
                <span aria-hidden className="w-2 h-2 bg-alert" />
                <span className="text-[10px] text-ink-faint lowercase">
                  error
                </span>
              </div>
            </TooltipTrigger>
            <TooltipContent>spans with status=error</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <div className="flex items-center gap-1">
                <span aria-hidden className="w-2 h-2 bg-warn" />
                <span className="text-[10px] text-ink-faint lowercase">
                  unset
                </span>
              </div>
            </TooltipTrigger>
            <TooltipContent>spans with status=unset</TooltipContent>
          </Tooltip>
        </div>

        {/* zoom controls */}
        <div className="flex items-center gap-1">
          {zoomLevel > 1 && (
            <span className="text-[10px] font-mono text-ink-faint mr-1 tabular-nums lowercase">
              {Math.round(zoomLevel * 100)}%
            </span>
          )}
          <button
            type="button"
            onClick={zoomOut}
            disabled={zoomLevel <= 1}
            className={cn(
              'p-1 text-ink-faint transition-colors',
              'hover:bg-bg hover:text-ink',
              'disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:text-ink-faint',
            )}
            title="zoom out"
            aria-label="zoom out"
          >
            <Minus className="w-3 h-3" />
          </button>
          <button
            type="button"
            onClick={zoomIn}
            disabled={zoomLevel >= 20}
            className={cn(
              'p-1 text-ink-faint transition-colors',
              'hover:bg-bg hover:text-ink',
              'disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:text-ink-faint',
            )}
            title="zoom in"
            aria-label="zoom in"
          >
            <Plus className="w-3 h-3" />
          </button>
          <button
            type="button"
            onClick={handleReset}
            disabled={zoomLevel === 1 && panOffset === 0}
            className={cn(
              'p-1 text-ink-faint transition-colors',
              'hover:bg-bg hover:text-ink',
              'disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:text-ink-faint',
            )}
            title="reset view"
            aria-label="reset view"
          >
            <RotateCcw className="w-3 h-3" />
          </button>
        </div>
      </div>

      {/* minimap (only when zoomed) */}
      {zoomLevel > 1 && (
        <div className="border-b border-rule flex-shrink-0">
          <canvas
            ref={minimapRef}
            onClick={handleMinimapClick}
            className="cursor-pointer w-full"
            style={{ height: MINIMAP_HEIGHT }}
          />
        </div>
      )}

      {/* canvas */}
      <div ref={containerRef} className="relative flex-1 overflow-hidden">
        <canvas
          ref={canvasRef}
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
          onClick={handleClick}
          onWheel={handleWheel}
          className="cursor-pointer"
        />
      </div>

      {/* tooltip — schematic panel, no shadows, no rounding */}
      {hoveredNode && (
        <div
          className="fixed z-50 pointer-events-none"
          style={{
            left: Math.min(tooltipPos.x + 12, window.innerWidth - 280),
            top: tooltipPos.y + 12,
          }}
        >
          <div className="bg-panel border border-rule px-3 py-2.5 min-w-[200px] max-w-[280px]">
            {/* span name */}
            <div className="font-mono text-[12.5px] text-ink leading-tight mb-1.5 break-all lowercase">
              {hoveredNode.span.name}
            </div>

            {/* service line */}
            <div className="flex items-center gap-1.5 mb-2">
              <StatusDot tone={hoveredStatusTone} />
              <span className="text-[10px] text-ink-faint font-mono lowercase">
                {hoveredNode.span.service_name ||
                  hoveredNode.span.name.split('.')[0]}
              </span>
            </div>

            {/* stats */}
            <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[10px]">
              <div className="flex justify-between">
                <span className="text-ink-faint lowercase">duration</span>
                <span className="text-accent font-mono tabular-nums">
                  {formatDuration(hoveredNode.span.duration_ms)}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-ink-faint lowercase">self</span>
                <span className="text-ink font-mono tabular-nums">
                  {formatDuration(hoveredNode.selfTime)}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-ink-faint lowercase">% trace</span>
                <span className="text-ink font-mono tabular-nums">
                  {tracePercent}%
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-ink-faint lowercase">status</span>
                <span
                  className={cn(
                    'font-mono lowercase',
                    hoveredNode.span.status === 'error' && 'text-alert',
                    hoveredNode.span.status === 'ok' && 'text-accent',
                    hoveredNode.span.status === 'unset' && 'text-warn',
                  )}
                >
                  {hoveredNode.span.status}
                </span>
              </div>
            </div>

            {/* trace percentage bar — single accent moment */}
            <div className="mt-2 h-1 bg-rule-2 overflow-hidden">
              <div
                className="h-full bg-accent transition-all duration-150"
                style={{
                  width: `${Math.max(1, Number.parseFloat(tracePercent))}%`,
                  opacity: 0.7,
                }}
              />
            </div>

            {/* children info */}
            {hoveredNode.children.length > 0 && (
              <div className="mt-1.5 text-[9px] text-ink-ghost lowercase tabular-nums">
                {hoveredNode.children.length} child span
                {hoveredNode.children.length === 1 ? '' : 's'}
              </div>
            )}
          </div>
        </div>
      )}

      {/* keyboard hint */}
      {zoomLevel <= 1 && (
        <div className="flex-shrink-0 px-3 py-1.5 text-[9px] text-ink-ghost text-center border-t border-rule lowercase tracking-[0.06em]">
          ctrl+scroll to zoom · scroll to pan · click to select
        </div>
      )}
    </div>
  )
}
