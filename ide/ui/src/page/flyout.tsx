/* Menus that open beside a menu, level with the row they come from, the
   way a desktop menu's submenus do: the branch menu's actions for a branch,
   and from there the ones for the branch it tracks. */

import { type PointerEvent, type ReactNode, useEffect, useLayoutEffect, useRef } from 'react'

/** The narrowest a menu gets beside another; with less room on both sides, its owner shows it in place. */
export const FLYOUT_MIN = 240
/** How long the pointer rests on a row before its menu opens, or replaces
    the one open: long enough to cross other rows on the way to it. */
const HOVER_MS = 200

export interface Beside {
  side: 'right' | 'left'
  /** The row's top, from the top of the menu it comes from. */
  top: number
  /** The room on that side. */
  room: number
}

/** Where a menu opens beside `menu` for `row`: the right when it fits, else
    the left; null when neither does. */
export function besideOf(menu: HTMLElement, row: HTMLElement): Beside | null {
  const box = menu.getBoundingClientRect()
  const right = window.innerWidth - box.right - 8
  const left = box.left - 8
  const top = row.getBoundingClientRect().top - box.top
  if (right >= FLYOUT_MIN) return { side: 'right', top, room: right }
  if (left >= FLYOUT_MIN) return { side: 'left', top, room: left }
  return null
}

/** A menu beside the positioned element it sits in, its first row level
    with `top`, kept on screen. */
export function Flyout({
  at,
  focus,
  onPointerEnter,
  children,
}: {
  at: Beside
  /** Opened from the keyboard: its first enabled row takes the focus. */
  focus: boolean
  onPointerEnter: () => void
  children: ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)
  // Past the other menu's border, and its own border and padding.
  const top = at.top - 6
  // Every render: a form or a file list opening changes its height.
  useLayoutEffect(() => {
    const element = ref.current
    if (element === null) return
    element.style.top = `${top}px`
    const over = element.getBoundingClientRect().bottom - (window.innerHeight - 8)
    if (over <= 0) return
    const parentTop = (element.offsetParent as HTMLElement | null)?.getBoundingClientRect().top ?? 0
    element.style.top = `${Math.max(top - over, 8 - parentTop)}px`
  })
  useEffect(() => {
    if (focus) ref.current?.querySelector<HTMLElement>('[data-list-item]:not([aria-disabled="true"])')?.focus()
  }, [focus])
  return (
    // The dropdown keeps Tab from moving focus; let it reach the rows.
    // biome-ignore lint/a11y/noStaticElementInteractions: only lets Tab through to the controls inside
    <div
      ref={ref}
      className="shui-wt-flyout"
      data-side={at.side}
      style={{ maxWidth: Math.min(at.room, 384) }}
      onPointerEnter={onPointerEnter}
      onKeyDown={(event) => event.key === 'Tab' && event.stopPropagation()}
    >
      {children}
    </div>
  )
}

/** Rows that open a menu beside theirs when the pointer rests on them: the
    `onRest` given to `listProps` gets the row under the pointer (matching
    `selector`) once it has stayed `HOVER_MS`, null for anything else in the
    list. A mouse only. `cancel`, on entering the menu beside, drops what is
    pending. */
export function useHoverIntent() {
  const timer = useRef<number | undefined>(undefined)
  const last = useRef<HTMLElement | null>(null)
  const latest = useRef<(row: HTMLElement | null) => void>(() => {})
  useEffect(() => () => window.clearTimeout(timer.current), [])
  const cancel = () => {
    window.clearTimeout(timer.current)
    last.current = null
  }
  const listProps = (selector: string, onRest: (row: HTMLElement | null) => void) => {
    latest.current = onRest
    return {
      onPointerOver: (event: PointerEvent<HTMLElement>) => {
        if (event.pointerType !== 'mouse') return
        const row = (event.target as Element).closest<HTMLElement>(selector)
        if (row === last.current) return
        last.current = row
        window.clearTimeout(timer.current)
        timer.current = window.setTimeout(() => latest.current(row), HOVER_MS)
      },
      onPointerLeave: cancel,
    }
  }
  return { cancel, listProps }
}
