import { useCallback, useEffect, useRef, useState } from 'react'

const TRACE_PANEL_DEFAULT = 520
const SPAN_PANEL_DEFAULT = 400
export const PANEL_MIN_WIDTH = 280
/**
 * Reserve kept for the trace list that flexes next to the trace/span
 * panels. The panels can grow as wide as the user drags, but never wider
 * than what leaves at least `PANEL_NEIGHBOR_MIN_WIDTH` pixels for the list.
 */
export const PANEL_NEIGHBOR_MIN_WIDTH = 240

/** Matches `className="w-[3px]"` on each separator handle in Traces/index.tsx. */
const HANDLE_WIDTH = 3
const FALLBACK_CONTAINER_WIDTH = 1200

export interface PanelWidths {
  trace: number
  span: number
}

export interface PanelMaxes {
  trace: number
  span: number
}

export interface UseResizablePanelsOptions {
  selectedSpanId: string | null
  containerRef: React.RefObject<HTMLDivElement | null>
}

export interface UseResizablePanelsReturn {
  panelWidths: PanelWidths
  panelMaxes: PanelMaxes
  panelMin: number
  isResizing: boolean
  startResize: (e: React.MouseEvent, panel: 'trace' | 'span') => void
  resetTracePanel: () => void
  resetSpanPanel: () => void
}

function getContainerWidth(
  ref: React.RefObject<HTMLDivElement | null>,
): number {
  return ref.current?.offsetWidth ?? FALLBACK_CONTAINER_WIDTH
}

export function useResizablePanels({
  selectedSpanId,
  containerRef,
}: UseResizablePanelsOptions): UseResizablePanelsReturn {
  const [panelWidths, setPanelWidths] = useState<PanelWidths>({
    trace: TRACE_PANEL_DEFAULT,
    span: SPAN_PANEL_DEFAULT,
  })
  const [isResizing, setIsResizing] = useState(false)
  const [containerWidth, setContainerWidth] = useState<number>(
    FALLBACK_CONTAINER_WIDTH,
  )

  const panelWidthsRef = useRef({
    trace: TRACE_PANEL_DEFAULT,
    span: SPAN_PANEL_DEFAULT,
  })
  panelWidthsRef.current = panelWidths

  const selectedSpanIdRef = useRef<string | null>(selectedSpanId)
  selectedSpanIdRef.current = selectedSpanId

  const isResizingRef = useRef<'trace' | 'span' | null>(null)
  const resizeStartRef = useRef({
    x: 0,
    width: 0,
    otherWidth: 0,
    containerWidth: FALLBACK_CONTAINER_WIDTH,
    hasSpan: false,
  })
  const prevSelectedSpanRef = useRef<string | null>(null)
  const preSplitTraceWidthRef = useRef(TRACE_PANEL_DEFAULT)

  // Auto-split panels when span detail opens, restore when it closes.
  // The split now reserves PANEL_NEIGHBOR_MIN_WIDTH for the list so opening
  // a span never crushes it off-screen.
  useEffect(() => {
    const hadSpan = prevSelectedSpanRef.current !== null
    const hasSpan = selectedSpanId !== null

    if (hasSpan && !hadSpan) {
      preSplitTraceWidthRef.current = panelWidthsRef.current.trace
      const cw = getContainerWidth(containerRef)
      const handlesWidth = HANDLE_WIDTH * 2
      const availableForPanels = cw - PANEL_NEIGHBOR_MIN_WIDTH - handlesWidth
      const halfWidth = Math.max(
        PANEL_MIN_WIDTH,
        Math.floor(availableForPanels / 2),
      )
      setPanelWidths({ trace: halfWidth, span: halfWidth })
    } else if (!hasSpan && hadSpan) {
      setPanelWidths((p) => ({ ...p, trace: preSplitTraceWidthRef.current }))
    }

    prevSelectedSpanRef.current = selectedSpanId
  }, [selectedSpanId, containerRef])

  const startResize = useCallback(
    (e: React.MouseEvent, panel: 'trace' | 'span') => {
      e.preventDefault()
      isResizingRef.current = panel
      setIsResizing(true)
      resizeStartRef.current = {
        x: e.clientX,
        width:
          panel === 'trace'
            ? panelWidthsRef.current.trace
            : panelWidthsRef.current.span,
        otherWidth:
          panel === 'trace'
            ? panelWidthsRef.current.span
            : panelWidthsRef.current.trace,
        containerWidth: getContainerWidth(containerRef),
        hasSpan: selectedSpanIdRef.current !== null,
      }
      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'
    },
    [containerRef],
  )

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!isResizingRef.current) return
      const dx = resizeStartRef.current.x - e.clientX

      if (isResizingRef.current === 'trace') {
        /* Uncoupled with the list: the trace panel can grow as wide as the
           container allows, leaving PANEL_NEIGHBOR_MIN_WIDTH for the list
           and (when present) the span panel's current width untouched. */
        const handles = resizeStartRef.current.hasSpan
          ? HANDLE_WIDTH * 2
          : HANDLE_WIDTH
        const otherForTrace = resizeStartRef.current.hasSpan
          ? resizeStartRef.current.otherWidth
          : 0
        const maxTrace = Math.max(
          PANEL_MIN_WIDTH,
          resizeStartRef.current.containerWidth -
            PANEL_NEIGHBOR_MIN_WIDTH -
            handles -
            otherForTrace,
        )
        const nextTrace = Math.max(
          PANEL_MIN_WIDTH,
          Math.min(maxTrace, resizeStartRef.current.width + dx),
        )
        setPanelWidths((p) => ({ ...p, trace: nextTrace }))
      } else {
        /* Coupled with the trace panel: total stays constant, both ≥
           PANEL_MIN_WIDTH. No upper cap is needed because the total is
           bounded by what the trace handle already set. */
        const totalWidth =
          resizeStartRef.current.width + resizeStartRef.current.otherWidth
        const maxForPanel = totalWidth - PANEL_MIN_WIDTH
        const newSpanWidth = Math.max(
          PANEL_MIN_WIDTH,
          Math.min(maxForPanel, resizeStartRef.current.width + dx),
        )
        const newTraceWidth = totalWidth - newSpanWidth
        setPanelWidths({ trace: newTraceWidth, span: newSpanWidth })
      }
    }
    const onMouseUp = () => {
      if (isResizingRef.current) {
        isResizingRef.current = null
        setIsResizing(false)
        document.body.style.cursor = ''
        document.body.style.userSelect = ''
      }
    }
    document.addEventListener('mousemove', onMouseMove)
    document.addEventListener('mouseup', onMouseUp)
    return () => {
      document.removeEventListener('mousemove', onMouseMove)
      document.removeEventListener('mouseup', onMouseUp)
    }
  }, [])

  /* Track container width + re-clamp panel widths on every viewport resize
     so the panels shrink to fit a narrower window. If even the minimums
     no longer fit (very narrow viewport), the panels stay at min and the
     parent container's overflow handles it. */
  useEffect(() => {
    if (typeof window === 'undefined') return
    const sync = () => {
      const cw = getContainerWidth(containerRef)
      setContainerWidth(cw)
      setPanelWidths((p) => {
        const hasSpan = selectedSpanIdRef.current !== null
        const handles = hasSpan ? HANDLE_WIDTH * 2 : HANDLE_WIDTH
        const otherForTrace = hasSpan ? p.span : 0
        const maxTrace = Math.max(
          PANEL_MIN_WIDTH,
          cw - PANEL_NEIGHBOR_MIN_WIDTH - handles - otherForTrace,
        )
        const nextTrace = Math.max(
          PANEL_MIN_WIDTH,
          Math.min(maxTrace, p.trace),
        )
        let nextSpan = p.span
        if (hasSpan) {
          const maxSpan = Math.max(
            PANEL_MIN_WIDTH,
            cw - PANEL_NEIGHBOR_MIN_WIDTH - handles - nextTrace,
          )
          nextSpan = Math.max(PANEL_MIN_WIDTH, Math.min(maxSpan, p.span))
        }
        if (nextTrace === p.trace && nextSpan === p.span) return p
        return { trace: nextTrace, span: nextSpan }
      })
    }
    sync()
    window.addEventListener('resize', sync)
    return () => window.removeEventListener('resize', sync)
  }, [containerRef])

  const resetTracePanel = useCallback(() => {
    setPanelWidths((p) => ({ ...p, trace: TRACE_PANEL_DEFAULT }))
  }, [])

  const resetSpanPanel = useCallback(() => {
    setPanelWidths((p) => ({ ...p, span: SPAN_PANEL_DEFAULT }))
  }, [])

  /* Dynamic caps for a11y (aria-valuemax). traceMax depends on the live
     container width and span width; spanMax is bounded by the coupled
     total since the span handle preserves trace+span. */
  const hasSpan = selectedSpanId !== null
  const handles = hasSpan ? HANDLE_WIDTH * 2 : HANDLE_WIDTH
  const traceMax = Math.max(
    PANEL_MIN_WIDTH,
    containerWidth -
      PANEL_NEIGHBOR_MIN_WIDTH -
      handles -
      (hasSpan ? panelWidths.span : 0),
  )
  const spanMax = hasSpan
    ? Math.max(
        PANEL_MIN_WIDTH,
        panelWidths.trace + panelWidths.span - PANEL_MIN_WIDTH,
      )
    : PANEL_MIN_WIDTH

  return {
    panelWidths,
    panelMaxes: { trace: traceMax, span: spanMax },
    panelMin: PANEL_MIN_WIDTH,
    isResizing,
    startResize,
    resetTracePanel,
    resetSpanPanel,
  }
}
