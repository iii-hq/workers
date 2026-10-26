import { ChevronLeft } from 'lucide-react'
import { type HTMLMotionProps, motion } from 'motion/react'
import * as React from 'react'
import { cn } from '@/lib/utils'

function hasContent(value: React.ReactNode): boolean {
  return React.Children.toArray(value).length > 0
}

/* The settings page vocabulary, after Linear: a plain title at the top of a
   left-aligned column, sections with a medium-weight heading, and hairline
   divided rows inside one bordered group. Inter throughout; 18px for the
   page title, 16px for section headings, 14px for everything else, with the
   neutral ramp carrying the hierarchy. The column is a container, so the
   gutters widen with the pane rather than the viewport. */

const GUTTER = 'px-5 @3xl:px-14'

export const SETTINGS_COLUMN = 'w-full max-w-[1120px]'

/* A settings search result lands on its row and tints it for a moment
   (`data-search-flash`, set by the search); the tint runs past the row's
   edges as a side-only shadow so the hairlines between rows stay put. */
const SEARCH_FLASH =
  'transition-[background-color,box-shadow] duration-500 ease-out [--search-flash:var(--color-neutral-100)] dark:[--search-flash:var(--color-neutral-900)]'

interface SettingsPageHeaderProps {
  title: React.ReactNode
  /** Sits after the title on the same line: an id crumb, a dirty dot. */
  titleAside?: React.ReactNode
  description?: React.ReactNode
  /** Narrow flow only: return to the settings list. */
  onBack?: () => void
  className?: string
}

export function SettingsPageHeader({
  title,
  titleAside,
  description,
  onBack,
  className,
}: SettingsPageHeaderProps) {
  return (
    <div
      className={cn(
        SETTINGS_COLUMN,
        GUTTER,
        'shrink-0 pt-6 pb-2 @3xl:pt-12',
        className,
      )}
    >
      {onBack ? (
        <button
          type="button"
          onClick={onBack}
          className="-ml-1.5 mb-3 inline-flex h-8 items-center gap-1 rounded-md pr-2 pl-1 text-sm text-muted-foreground transition-colors duration-150 ease-out outline-none hover:bg-neutral-100 hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 pointer-coarse:h-11 dark:hover:bg-neutral-900"
        >
          <ChevronLeft className="size-4" aria-hidden />
          Settings
        </button>
      ) : null}
      <div className="flex min-w-0 items-center gap-2.5">
        <h1 className="truncate text-lg font-semibold leading-7 tracking-[-0.02em] text-foreground">
          {title}
        </h1>
        {titleAside}
      </div>
      {hasContent(description) ? (
        <p className="mt-1 text-pretty text-sm leading-5 text-muted-foreground">
          {description}
        </p>
      ) : null}
    </div>
  )
}

export function SettingsPageBody({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        SETTINGS_COLUMN,
        GUTTER,
        'flex flex-col gap-12 pt-6 pb-24 @3xl:pt-8',
        className,
      )}
      {...props}
    />
  )
}

export interface SettingsSectionProps
  extends Omit<React.HTMLAttributes<HTMLElement>, 'title'> {
  title: React.ReactNode
  description?: React.ReactNode
}

export function SettingsSection({
  title,
  description,
  className,
  children,
  ...props
}: SettingsSectionProps) {
  const id = React.useId()
  const titleId = `${id}-title`
  const descriptionId = `${id}-description`
  const hasDescription = hasContent(description)
  return (
    <section
      aria-labelledby={titleId}
      aria-describedby={hasDescription ? descriptionId : undefined}
      className={cn('flex min-w-0 scroll-mt-6 flex-col', className)}
      {...props}
    >
      <h2
        id={titleId}
        className={cn(
          'rounded-sm text-base font-medium leading-6 text-foreground',
          SEARCH_FLASH,
          '[[data-search-flash]>&]:bg-(--search-flash) [[data-search-flash]>&]:shadow-[-8px_0_0_var(--search-flash),8px_0_0_var(--search-flash)]',
        )}
      >
        {title}
      </h2>
      {hasDescription ? (
        <p
          id={descriptionId}
          className="mt-1 text-pretty text-sm leading-5 text-muted-foreground"
        >
          {description}
        </p>
      ) : null}
      <div className="mt-5 flex min-w-0 flex-col gap-4">{children}</div>
    </section>
  )
}

/** The bordered group that holds a section's rows, divided by hairlines. */
export function SettingsList({
  className,
  // `list-none` strips native list semantics in Safari/VoiceOver; the explicit
  // role restores them.
  role = 'list',
  ...props
}: React.HTMLAttributes<HTMLUListElement>) {
  return (
    <ul
      role={role}
      className={cn(
        'min-w-0 list-none border-y border-neutral-200 dark:border-neutral-800',
        '[&>*+*]:border-t [&>*+*]:border-neutral-200 dark:[&>*+*]:border-neutral-800',
        className,
      )}
      {...props}
    />
  )
}

export type SettingsRowLayout = 'auto' | 'inline' | 'stacked'

export interface SettingsRowProps
  extends Omit<HTMLMotionProps<'li'>, 'children' | 'title' | 'layout'> {
  label: React.ReactNode
  description?: React.ReactNode
  /** A quiet current value under the description, in tabular figures. */
  meta?: React.ReactNode
  /** The row's control, on the trailing edge. */
  control?: React.ReactNode
  /** A secondary action composed after `control`. */
  action?: React.ReactNode
  /** `auto` stacks the trailing slot when the row is narrower than 30rem. */
  layout?: SettingsRowLayout
}

/**
 * One setting: label and description on the left, its control on the right.
 * A `motion.li`, so a row that appears conditionally can fade in; rows that
 * are always there render still (`initial={false}` is the default).
 */
export function SettingsRow({
  label,
  description,
  meta,
  control,
  action,
  layout = 'auto',
  className,
  initial = false,
  ...props
}: SettingsRowProps) {
  const hasTrailing = hasContent(control) || hasContent(action)
  const twoColumns = hasTrailing && layout !== 'stacked'
  return (
    <motion.li
      data-layout={layout}
      initial={initial}
      className={cn(
        '@container min-w-0',
        SEARCH_FLASH,
        'data-search-flash:bg-(--search-flash) data-search-flash:shadow-[-12px_0_0_var(--search-flash),12px_0_0_var(--search-flash)]',
        className,
      )}
      {...props}
    >
      <div
        className={cn(
          'flex min-h-[72px] min-w-0 flex-col gap-4 py-5',
          twoColumns && layout === 'inline'
            ? 'flex-row items-center justify-between gap-6'
            : twoColumns
              ? '@[30rem]:flex-row @[30rem]:items-center @[30rem]:justify-between @[30rem]:gap-6'
              : null,
        )}
      >
        <div className="min-w-0 flex-1">
          <div className="text-base font-medium leading-6 text-foreground [overflow-wrap:anywhere]">
            {label}
          </div>
          {hasContent(description) ? (
            <div className="mt-1 text-sm leading-5 text-muted-foreground [overflow-wrap:anywhere]">
              {description}
            </div>
          ) : null}
          {hasContent(meta) ? (
            <div className="mt-1 text-sm leading-5 tabular-nums text-neutral-400 dark:text-neutral-500">
              {meta}
            </div>
          ) : null}
        </div>
        {hasTrailing ? (
          <div className="flex min-w-0 max-w-full shrink-0 items-center gap-2">
            {control}
            {action}
          </div>
        ) : null}
      </div>
    </motion.li>
  )
}
