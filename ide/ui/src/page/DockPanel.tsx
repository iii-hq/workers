/* The workspace frame's docked panel: the section a tool window sits in at
   the frame's bottom or right edge, with the drag handle that sizes it. The
   terminal and the Git window take turns in it, and share its classes. */

import { useSplitDrag } from '@iii-dev/console-ui/hooks'
import { type CSSProperties, type ReactNode, useEffect, useRef, useState } from 'react'
import type { TerminalDock } from './persist'

interface ResizeState {
  startSize: number
  maxSize: number
}

function clampSize(size: number, maxSize: number): number {
  return Math.min(Math.max(160, maxSize), Math.max(160, Math.round(size)))
}

function dockStyle(dock: TerminalDock, size: number): CSSProperties | undefined {
  switch (dock) {
    case 'bottom':
      return { height: size }
    case 'right':
      return { width: size }
    case 'editor':
      return undefined
    default: {
      const exhaustive: never = dock
      return exhaustive
    }
  }
}

export function DockPanel({
  dock,
  size,
  narrow,
  maximized = false,
  hidden = false,
  label,
  noun,
  onSizeChange,
  children,
}: {
  dock: TerminalDock
  size: number
  /** The page is narrow: a right dock stacks under the editor at full width,
      with no resize handle. */
  narrow?: boolean
  /** Fills the whole frame over the editor, which stays mounted beneath. */
  maximized?: boolean
  /** Kept mounted out of sight, so what it holds keeps its state. */
  hidden?: boolean
  /** The panel's accessible name. */
  label: string
  /** What the handle resizes, as it reads in "Resize bottom <noun>". */
  noun: string
  onSizeChange: (size: number) => void
  children: ReactNode
}) {
  const panelRef = useRef<HTMLElement>(null)
  const [resizeBounds, setResizeBounds] = useState({ size, max: 1200 })
  // A narrow page's right dock takes its size from styles.css, stacked under
  // the editor at full width: it has no handle, as a drag could not size it.
  const docked = dock !== 'editor' && !maximized && !(narrow && dock === 'right')
  // A drag sizes the panel here and tells the page once, on release: the
  // page re-rendering on every pointer move would redraw everything else.
  const [dragSize, setDragSize] = useState<number | null>(null)
  const dragSizeRef = useRef<number | null>(null)
  const style = docked ? dockStyle(dock, dragSize ?? size) : undefined

  useEffect(() => {
    const panel = panelRef.current
    const frame = panel?.parentElement
    if (!panel || !frame || !docked) return
    const update = () => {
      const panelRect = panel.getBoundingClientRect()
      const frameRect = frame.getBoundingClientRect()
      setResizeBounds({
        size: dock === 'bottom' ? panelRect.height : panelRect.width,
        max: dock === 'bottom' ? Math.max(160, frameRect.height - 120) : Math.max(160, frameRect.width - 240),
      })
    }
    update()
    const observer = new ResizeObserver(update)
    observer.observe(panel)
    observer.observe(frame)
    return () => observer.disconnect()
  }, [dock, docked])

  // Maximized, it covers the rest of the frame: keep the keyboard and
  // assistive tech out of what it hides.
  useEffect(() => {
    const panel = panelRef.current
    if (!maximized || hidden || !panel?.parentElement) return
    const covered = [...panel.parentElement.children].filter((child) => child !== panel && !child.hasAttribute('inert'))
    for (const child of covered) child.setAttribute('inert', '')
    return () => {
      for (const child of covered) child.removeAttribute('inert')
    }
  }, [maximized, hidden])

  const maxSizeOf = (frame: Element | null | undefined) => {
    const rect = frame?.getBoundingClientRect()
    return dock === 'bottom'
      ? Math.max(160, (rect?.height ?? window.innerHeight) - 120)
      : Math.max(160, (rect?.width ?? window.innerWidth) - 240)
  }
  const currentSizeOf = (panel: Element | null) => {
    const rect = panel?.getBoundingClientRect()
    return dock === 'bottom' ? (rect?.height ?? size) : (rect?.width ?? size)
  }
  const resizer = useSplitDrag<ResizeState>({
    horizontal: dock === 'right',
    begin: (event) => {
      if (!docked) return null
      const panel = event.currentTarget.parentElement
      return { startSize: currentSizeOf(panel), maxSize: maxSizeOf(panel?.parentElement) }
    },
    move: (origin, delta) => {
      const next = clampSize(origin.startSize - delta, origin.maxSize)
      dragSizeRef.current = next
      setDragSize(next)
    },
    // Up/Left grow the panel: its free edge faces the start of the axis.
    step: (direction, event) => {
      const panel = event.currentTarget.parentElement
      onSizeChange(clampSize(currentSizeOf(panel) + (direction === -1 ? 16 : -16), maxSizeOf(panel?.parentElement)))
    },
  })

  const commitDrag = () => {
    const next = dragSizeRef.current
    if (next === null) return
    dragSizeRef.current = null
    onSizeChange(next)
    setDragSize(null)
  }

  return (
    <section
      ref={panelRef}
      className="shui-terminal-panel"
      data-terminal-dock={dock}
      data-maximized={maximized || undefined}
      hidden={hidden}
      style={style}
      aria-label={label}
    >
      {docked ? (
        // biome-ignore lint/a11y/useSemanticElements: this is an interactive range separator, not a static thematic break.
        <div
          role="separator"
          tabIndex={0}
          className="shui-terminal-resize"
          {...resizer}
          onPointerUp={(event) => {
            resizer.onPointerUp(event)
            commitDrag()
          }}
          onPointerCancel={(event) => {
            resizer.onPointerCancel(event)
            commitDrag()
          }}
          onLostPointerCapture={(event) => {
            resizer.onLostPointerCapture(event)
            commitDrag()
          }}
          aria-label={`Resize ${dock} ${noun}`}
          aria-orientation={dock === 'bottom' ? 'horizontal' : 'vertical'}
          aria-valuemin={160}
          aria-valuemax={Math.round(resizeBounds.max)}
          aria-valuenow={Math.round(resizeBounds.size)}
          title={`Drag to resize ${noun}`}
        >
          <span aria-hidden />
        </div>
      ) : null}
      {children}
    </section>
  )
}
