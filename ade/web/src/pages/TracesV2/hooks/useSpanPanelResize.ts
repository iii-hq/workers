/**
 * Width state + drag handling for the span detail panel — the single
 * resizable split left in TracesV2 now that the trace detail fills the
 * canvas (the old 3-panel list/trace/span maths lived in
 * `useResizablePanels`, deleted with the list split).
 */

import { useCallback, useEffect, useRef, useState } from 'react'

const SPAN_PANEL_DEFAULT = 400
export const SPAN_PANEL_MIN = 280
/** room always kept for the trace detail next to the panel */
const NEIGHBOR_MIN_WIDTH = 360
const FALLBACK_CONTAINER_WIDTH = 1200

export interface UseSpanPanelResizeReturn {
  width: number
  min: number
  max: number
  isResizing: boolean
  startResize: (e: React.MouseEvent) => void
  reset: () => void
}

export function useSpanPanelResize(
  containerRef: React.RefObject<HTMLDivElement | null>,
): UseSpanPanelResizeReturn {
  const [width, setWidth] = useState(SPAN_PANEL_DEFAULT)
  const [containerWidth, setContainerWidth] = useState(FALLBACK_CONTAINER_WIDTH)
  const [isResizing, setIsResizing] = useState(false)
  const dragRef = useRef<{ x: number; width: number } | null>(null)

  const max = Math.max(SPAN_PANEL_MIN, containerWidth - NEIGHBOR_MIN_WIDTH)
  const maxRef = useRef(max)
  maxRef.current = max

  const startResize = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      dragRef.current = { x: e.clientX, width }
      setIsResizing(true)
      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'
    },
    [width],
  )

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      const drag = dragRef.current
      if (!drag) return
      const next = drag.width + (drag.x - e.clientX)
      setWidth(Math.max(SPAN_PANEL_MIN, Math.min(maxRef.current, next)))
    }
    const onMouseUp = () => {
      if (!dragRef.current) return
      dragRef.current = null
      setIsResizing(false)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.addEventListener('mousemove', onMouseMove)
    document.addEventListener('mouseup', onMouseUp)
    return () => {
      document.removeEventListener('mousemove', onMouseMove)
      document.removeEventListener('mouseup', onMouseUp)
    }
  }, [])

  // Track the container and re-clamp on every viewport resize.
  useEffect(() => {
    if (typeof window === 'undefined') return
    const sync = () => {
      const cw = containerRef.current?.offsetWidth ?? FALLBACK_CONTAINER_WIDTH
      setContainerWidth(cw)
      setWidth((w) =>
        Math.max(
          SPAN_PANEL_MIN,
          Math.min(w, Math.max(SPAN_PANEL_MIN, cw - NEIGHBOR_MIN_WIDTH)),
        ),
      )
    }
    sync()
    window.addEventListener('resize', sync)
    return () => window.removeEventListener('resize', sync)
  }, [containerRef])

  const reset = useCallback(() => setWidth(SPAN_PANEL_DEFAULT), [])

  return { width, min: SPAN_PANEL_MIN, max, isResizing, startResize, reset }
}
