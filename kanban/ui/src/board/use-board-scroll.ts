import { useEffect, useState } from 'react'

/** Where the board can still scroll: drives the edge fades. */
export type ScrollEdges = { left: boolean; right: boolean }

/** How close to an edge (px) a dragged card starts the board moving, and its top speed (px per frame). */
const EDGE_ZONE = 96
const EDGE_MAX_SPEED = 18
/** A press has to travel this far before it pans, so a click on the board stays a click. */
const PAN_SLOP = 4

/** Anything a press on the board should leave alone: cards (they drag), controls, open menus. */
const INTERACTIVE = '.kb-card, button, a, input, textarea, select, [role="menu"], [role="listbox"], [role="dialog"]'

/**
 * The horizontal board, made to feel like Linear's:
 * - a plain wheel scrolls the board sideways wherever the pointer is not over a column that scrolls itself;
 * - pressing and dragging the empty board pans it;
 * - a card dragged near the left or right edge carries the board with it, faster the closer it gets;
 * - `edges` says which sides still have columns out of view, for the fades.
 */
export function useBoardScroll(dragging: boolean): { ref: (node: HTMLElement | null) => void; edges: ScrollEdges } {
  const [el, setEl] = useState<HTMLElement | null>(null)
  const [edges, setEdges] = useState<ScrollEdges>({ left: false, right: false })

  useEffect(() => {
    if (!el) return
    const update = () => {
      const left = el.scrollLeft > 1
      const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1
      setEdges((prev) => (prev.left === left && prev.right === right ? prev : { left, right }))
    }
    update()
    el.addEventListener('scroll', update, { passive: true })
    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => {
      el.removeEventListener('scroll', update)
      observer.disconnect()
    }
  }, [el])

  // Wheel: vertical deltas become horizontal outside a column that can scroll. Trackpads' sideways swipes,
  // shift+wheel and pinch-zoom keep their own behaviour.
  useEffect(() => {
    if (!el) return
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.shiftKey) return
      if (Math.abs(event.deltaX) >= Math.abs(event.deltaY)) return
      if (el.scrollWidth <= el.clientWidth) return
      const body = (event.target as Element | null)?.closest<HTMLElement>('.kb-column__body')
      if (body && body.scrollHeight > body.clientHeight + 1) return
      event.preventDefault()
      el.scrollLeft += event.deltaY
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [el])

  // Press-and-drag on the empty board pans it (mouse only; touch already scrolls natively).
  useEffect(() => {
    if (!el) return
    let start: { x: number; left: number; id: number } | null = null
    const down = (event: PointerEvent) => {
      if (event.button !== 0 || event.pointerType !== 'mouse') return
      if ((event.target as Element | null)?.closest(INTERACTIVE)) return
      // A press on empty board would start a text selection, which the browser turns into a native drag (and
      // cancels the pointer) on the first move: this press is a pan, not a selection.
      event.preventDefault()
      start = { x: event.clientX, left: el.scrollLeft, id: event.pointerId }
    }
    const move = (event: PointerEvent) => {
      if (!start) return
      const dx = event.clientX - start.x
      if (el.dataset.panning !== 'true') {
        if (Math.abs(dx) < PAN_SLOP) return
        el.dataset.panning = 'true'
        el.setPointerCapture(start.id)
      }
      el.scrollLeft = start.left - dx
    }
    const up = () => {
      if (start && el.dataset.panning === 'true') {
        delete el.dataset.panning
        if (el.hasPointerCapture(start.id)) el.releasePointerCapture(start.id)
      }
      start = null
    }
    el.addEventListener('pointerdown', down)
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
    el.addEventListener('pointercancel', up)
    return () => {
      el.removeEventListener('pointerdown', down)
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      el.removeEventListener('pointercancel', up)
      delete el.dataset.panning
    }
  }, [el])

  // A card in the air near an edge scrolls the board toward the columns out of view.
  useEffect(() => {
    if (!el || !dragging) return
    let speed = 0
    let frame = 0
    const tick = () => {
      if (speed !== 0) el.scrollLeft += speed
      frame = requestAnimationFrame(tick)
    }
    frame = requestAnimationFrame(tick)
    const over = (event: DragEvent) => {
      const rect = el.getBoundingClientRect()
      const fromLeft = event.clientX - rect.left
      const fromRight = rect.right - event.clientX
      const ramp = (distance: number) => Math.ceil(((EDGE_ZONE - distance) / EDGE_ZONE) * EDGE_MAX_SPEED)
      speed = fromLeft < EDGE_ZONE ? -ramp(fromLeft) : fromRight < EDGE_ZONE ? ramp(fromRight) : 0
    }
    const stop = () => {
      speed = 0
    }
    el.addEventListener('dragover', over)
    el.addEventListener('dragleave', stop)
    window.addEventListener('drop', stop)
    window.addEventListener('dragend', stop)
    return () => {
      cancelAnimationFrame(frame)
      el.removeEventListener('dragover', over)
      el.removeEventListener('dragleave', stop)
      window.removeEventListener('drop', stop)
      window.removeEventListener('dragend', stop)
    }
  }, [el, dragging])

  return { ref: setEl, edges }
}
