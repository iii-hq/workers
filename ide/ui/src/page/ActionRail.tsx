/* The Git window's actions, WebStorm's way: one list per selection feeds
   the vertical icon rail on a pane's left edge and the row's context menu
   alike, so every action a menu offers can also be reached by touch. An
   action that cannot run now stays in place, disabled, and says why.

   A narrow pane lays the rail out as a bar along its bottom edge, each
   button with its short label under the icon, within a thumb's reach: the
   primary actions, and More for the rest. */

import { Toolbar, Tooltip } from '@iii-dev/console-ui'
import { Ellipsis } from 'lucide-react'
import { Fragment, type KeyboardEvent, memo, type ReactNode } from 'react'
import type { ContextMenuAnchor, ContextMenuItem } from './ContextMenu'

export interface GitAction {
  id: string
  label: string
  /** The label under its icon in the narrow bar; `label` when absent. */
  short?: string
  icon: ReactNode
  /** Key hint shown in the tooltip and the menu. Display only. */
  shortcut?: string
  /** Why it cannot run now; null or absent when it can. */
  blocked?: string | null
  danger?: boolean
  run(): void
  /** Where it shows: the rail and the menu (default), or one of them. */
  in?: 'rail' | 'menu'
  /** Sinks to the rail's far end. */
  end?: boolean
  /** A menu row that opens these (never on the rail). */
  items?: GitAction[]
  /** False when it makes no sense for the selection (a push for a tag):
      disabled on the rail, which keeps its places, and left out of menus. */
  applies?: boolean
  /** Stays in the narrow bar; the others go under More. */
  primary?: boolean
  /** Actions of one intent (open, sync, …): a new group starts with a
      divider on the rail and a separator in the menu. */
  group?: string
}

/** Where a new group starts: after the previous action shown, when both
    name a group and they differ. */
function startsGroup(action: GitAction, previous: GitAction | undefined): boolean {
  return (
    previous !== undefined &&
    action.group !== undefined &&
    previous.group !== undefined &&
    action.group !== previous.group
  )
}

function RailButton({ action, bar }: { action: GitAction; bar: boolean }) {
  const blocked = action.blocked ?? null
  const hint = action.shortcut ? ` (${action.shortcut})` : ''
  return (
    <Tooltip label={blocked === null ? `${action.label}${hint}` : `${action.label}: ${blocked}`}>
      <button
        type="button"
        className="shui-git-rail-action"
        aria-label={action.label}
        aria-disabled={blocked !== null || undefined}
        data-tone={action.danger ? 'alert' : undefined}
        onClick={() => {
          if (blocked === null) action.run()
        }}
      >
        {action.icon}
        {bar ? <span className="shui-git-rail-label">{action.short ?? action.label}</span> : null}
      </button>
    </Tooltip>
  )
}

/** The arrows along the rail step through its buttons, as in a toolbar. */
function railKeys(bar: boolean) {
  const [back, forward] = bar ? ['ArrowLeft', 'ArrowRight'] : ['ArrowUp', 'ArrowDown']
  return (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== back && event.key !== forward) return
    const buttons = [...event.currentTarget.querySelectorAll<HTMLElement>('.shui-git-rail-action')]
    const at = buttons.indexOf(document.activeElement as HTMLElement)
    if (at < 0) return
    event.preventDefault()
    buttons[(at + (event.key === forward ? 1 : -1) + buttons.length) % buttons.length]?.focus()
  }
}

/** Memoized: a caller that keeps its action list while the selection
    stays skips re-rendering every button. */
export const ActionRail = memo(function ActionRail({
  label,
  actions,
  bar = false,
  onMore,
}: {
  label: string
  actions: readonly GitAction[]
  /** Lay out as the narrow pane's bottom bar. */
  bar?: boolean
  /** The bar's More: the actions left out of it, to open as a menu. */
  onMore?(anchor: ContextMenuAnchor, rest: GitAction[]): void
}) {
  const shown = actions.filter((action) => action.in !== 'menu' && action.items === undefined)
  const button = (action: GitAction) => <RailButton key={action.id} action={action} bar={bar} />
  if (!bar) {
    const top = shown.filter((action) => !action.end)
    return (
      <Toolbar
        as="nav"
        orientation="vertical"
        aria-label={label}
        className="shui-git-rail"
        onKeyDown={railKeys(false)}
        end={shown.filter((action) => action.end).map(button)}
      >
        {top.map((action, index) => (
          <Fragment key={action.id}>
            {startsGroup(action, top[index - 1]) ? <span className="shui-git-rail-sep" aria-hidden /> : null}
            {button(action)}
          </Fragment>
        ))}
      </Toolbar>
    )
  }
  const primary = shown.some((action) => action.primary) ? shown.filter((action) => action.primary) : shown.slice(0, 4)
  const rest = actions.filter((action) => !primary.includes(action) && action.applies !== false)
  return (
    <Toolbar
      as="nav"
      orientation="horizontal"
      aria-label={label}
      className="shui-git-rail shui-git-actionbar"
      onKeyDown={railKeys(true)}
    >
      {primary.map(button)}
      {onMore && rest.length > 0 ? (
        <button
          type="button"
          className="shui-git-rail-action"
          aria-label="More actions"
          aria-haspopup="menu"
          onClick={(event) => {
            const rect = event.currentTarget.getBoundingClientRect()
            onMore({ x: rect.left, y: rect.top }, rest)
          }}
        >
          <Ellipsis aria-hidden />
          <span className="shui-git-rail-label">More</span>
        </button>
      ) : null}
    </Toolbar>
  )
})

/** The same actions as context menu rows: those that apply, and with
    `withRail` the rail's own (Fetch, Collapse all) too. A new group starts
    after a separator. */
export function menuItems(actions: readonly GitAction[], withRail = false): ContextMenuItem[] {
  const listed = actions.filter((action) => (withRail || action.in !== 'rail') && action.applies !== false)
  return listed.flatMap((action, index): ContextMenuItem[] => [
    ...(startsGroup(action, listed[index - 1]) ? [{ type: 'separator' as const, id: `sep:${action.id}` }] : []),
    action.items !== undefined
      ? {
          type: 'submenu',
          id: action.id,
          label: action.label,
          icon: action.icon,
          disabled: (action.blocked ?? null) !== null,
          items: menuItems(action.items),
        }
      : {
          id: action.id,
          label: action.label,
          icon: action.icon,
          shortcut: action.shortcut,
          disabled: (action.blocked ?? null) !== null,
          danger: action.danger,
          onSelect: action.run,
        },
  ])
}
