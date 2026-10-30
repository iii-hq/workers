/**
 * Page chrome — THE layout design system every workspace pane follows,
 * console-native screens (chat, traces) and worker-injected pages alike.
 * The whole set is exported through `@iii-dev/console-ui`, so a worker
 * page composes the exact same components (and picks up console-side
 * restyles for free) instead of re-approximating the look per worker.
 *
 * The vocabulary, one surface per role (see index.css's surface ramp):
 *
 *   PageShell    the pane's root column — fills the pane, `bg-panel`
 *   PageHeader   the standard top bar: `bg-panel-raised`, hairline edge
 *                border, icon + title + description, an actions slot,
 *                the hosting pane's split control, and the ✕ that closes
 *                the hosting pane (`onClose` — pages get it as
 *                `PageRenderProps.onRequestClose`)
 *   PageBody     the row under the header (`side` mirrors it so a
 *                sidebar can hug the pane's outer edge)
 *   PageSidebar  the navigation column — `bg-sidebar`, bounded width
 *   PageMain     the primary workspace — `bg-panel`
 *
 * PageBody separates its children with a 1px `bg-edge` gap instead of
 * per-child borders, so any composition (sidebar left, right, absent)
 * gets the same hairline without border bookkeeping.
 */

import { Ellipsis, Settings, X } from 'lucide-react'
import type * as React from 'react'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { PaneSplitMenu } from '@/components/workspace/PaneSplitMenu'
import {
  openPaneConfiguration,
  usePaneConfigurationId,
} from '@/lib/pane-configuration'
import { cn } from '@/lib/utils'
import { IconButton } from './IconButton'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

export { PageSidebar, type PageSidebarProps } from './PageSidebar'

/** The pane's root column. Fills the pane whether the parent is a flex
    column (native screens) or a block scroller (the ext-page host). */
export function PageShell({ className, ...rest }: React.ComponentProps<'div'>) {
  return (
    <div
      className={cn(
        '@container flex-1 h-full min-h-0 min-w-0 flex flex-col bg-panel',
        className,
      )}
      {...rest}
    />
  )
}

/** A header icon's phone touch target (48px), shared by Settings and ⋯. */
const HEADER_ICON_TOUCH = 'max-sm:size-12'

/** One row of a page header's ⋯ menu. */
export interface PageHeaderMenuItem {
  id: string
  label: string
  icon?: React.ReactNode
  /** Shown at the row's end, e.g. `⇧⌥G`. */
  shortcut?: string
  /** A toggle: the row shows a check while on. Absent: a plain action. */
  checked?: boolean
  disabled?: boolean
  onSelect(): void
}

export interface PageHeaderProps {
  /** Identity glyph, rendered at 16px in faint ink (any svg fits). */
  icon?: React.ReactNode
  /** The page's human-readable name. Technical identifiers belong in `description`. */
  title?: React.ReactNode
  /** One short descriptor; truncates before anything else gives. */
  description?: React.ReactNode
  /** Free-form middle content (fills the flexible gap before actions). */
  children?: React.ReactNode
  /** Right-side controls, rendered before the close affordance. */
  actions?: React.ReactNode
  /** Close the hosting pane — renders the standard ✕ when present.
      Worker pages: pass `PageRenderProps.onRequestClose` through. */
  onClose?: () => void
  /** The page's less frequent controls, behind one ⋯ button at the end of
      the actions. The pane's Settings joins the menu instead of standing
      on its own. */
  menu?: readonly PageHeaderMenuItem[]
  className?: string
}

/** The standard pane top bar: slightly raised, hairline bottom edge. */
export function PageHeader({
  icon,
  title,
  description,
  children,
  actions,
  onClose,
  menu,
  className,
}: PageHeaderProps) {
  const configurationId = usePaneConfigurationId()

  return (
    <header
      className={cn(
        'flex h-14 shrink-0 items-center gap-2.5 whitespace-nowrap px-3 sm:h-11',
        'bg-panel-raised border-b border-edge',
        className,
      )}
    >
      {icon ? (
        <span
          aria-hidden
          className="shrink-0 flex items-center text-ink-faint [&>svg]:size-4"
        >
          {icon}
        </span>
      ) : null}
      {title ? (
        <h1 className="shrink-0 font-sans text-[14px] font-medium text-ink">
          {title}
        </h1>
      ) : null}
      {description ? (
        <span className="min-w-0 truncate font-sans text-[12px] text-ink-faint">
          {description}
        </span>
      ) : null}
      <div className="flex-1 min-w-0 flex items-center gap-2.5">{children}</div>
      {actions ? (
        <div className="shrink-0 flex items-center gap-1.5">{actions}</div>
      ) : null}
      {/* The hosting pane's split control (a workspace pane on desktop
          only), the same in every pane's header. */}
      <PaneSplitMenu />
      {menu && menu.length > 0 ? (
        <PageHeaderMenu
          items={menu}
          onOpenSettings={
            configurationId
              ? () => void openPaneConfiguration(configurationId)
              : undefined
          }
        />
      ) : configurationId ? (
        <IconButton
          label="Settings"
          tooltipSide="bottom"
          className={HEADER_ICON_TOUCH}
          onClick={() => void openPaneConfiguration(configurationId)}
        >
          <Settings aria-hidden className="size-4 shrink-0" />
        </IconButton>
      ) : null}
      {/* No tooltip on the ✕ — a lone close glyph at a pane's corner is
          self-describing, and a box popping over the tab bar to say "close
          panel" costs more attention than it saves. aria-label still names
          it for screen readers. */}
      {onClose ? (
        <button
          type="button"
          onClick={onClose}
          aria-label="close panel"
          className="relative flex size-12 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus sm:size-7"
        >
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <X aria-hidden className="size-4 shrink-0" />
        </button>
      ) : null}
    </header>
  )
}

export interface PageBodyProps extends React.HTMLAttributes<HTMLDivElement> {
  /** The pane's side of the tab (`PageRenderProps.panelSide`) — `right`
      mirrors the row so the sidebar hugs the pane's outer edge. */
  side?: 'left' | 'right'
}

/** The row under the header. The 1px gap paints in `bg-edge`, giving a
    hairline between sidebar and main with zero border bookkeeping. */
export function PageBody({ side = 'left', className, ...rest }: PageBodyProps) {
  return (
    <div
      className={cn(
        'flex-1 min-h-0 min-w-0 flex gap-px bg-edge',
        side === 'right' && 'flex-row-reverse',
        className,
      )}
      {...rest}
    />
  )
}

/** The primary workspace column: white (`bg-panel`), takes what's left. */
export function PageMain({
  className,
  ...rest
}: React.HTMLAttributes<HTMLElement>) {
  return (
    <main
      className={cn(
        'flex-1 min-h-0 min-w-0 flex flex-col overflow-hidden bg-panel',
        className,
      )}
      {...rest}
    />
  )
}

/** The header's ⋯ menu: the page's rows, then Settings. */
function PageHeaderMenu({
  items,
  onOpenSettings,
}: {
  items: readonly PageHeaderMenuItem[]
  onOpenSettings?: () => void
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <IconButton
            label="More actions"
            tooltipSide="bottom"
            className={HEADER_ICON_TOUCH}
          >
            <Ellipsis aria-hidden className="size-4 shrink-0" />
          </IconButton>
        }
      />
      <DropdownMenuContent align="end" className="min-w-[14rem]">
        {items.map((item) =>
          item.checked === undefined ? (
            <DropdownMenuItem
              key={item.id}
              disabled={item.disabled}
              onClick={item.onSelect}
            >
              {item.icon}
              {item.label}
              {item.shortcut ? (
                <DropdownMenuShortcut>{item.shortcut}</DropdownMenuShortcut>
              ) : null}
            </DropdownMenuItem>
          ) : (
            <DropdownMenuCheckboxItem
              key={item.id}
              checked={item.checked}
              disabled={item.disabled}
              onCheckedChange={item.onSelect}
            >
              {item.icon}
              {item.label}
              {item.shortcut ? (
                <DropdownMenuShortcut>{item.shortcut}</DropdownMenuShortcut>
              ) : null}
            </DropdownMenuCheckboxItem>
          ),
        )}
        {onOpenSettings ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={onOpenSettings}>
              <Settings aria-hidden />
              Settings
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
