import { ArrowLeft, Check, ChevronRight } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCallback, useId, useState } from 'react'
import { cn } from '@/lib/utils'
import { BottomSheetDescription, BottomSheetTitle } from './BottomSheet'

/**
 * A tiny, presentation-agnostic history stack for sheets with drill-in pages.
 * Consumers own the page ids and render the active page, so navigation never
 * creates another portal, overlay, or dialog layer.
 */
export function useSheetNavigation<Page extends string>(rootPage: Page) {
  const [stack, setStack] = useState<readonly Page[]>([rootPage])
  const page = stack.at(-1) ?? rootPage

  const push = useCallback((next: Page) => {
    setStack((current) =>
      current.at(-1) === next ? current : [...current, next],
    )
  }, [])

  const back = useCallback(() => {
    setStack((current) => (current.length > 1 ? current.slice(0, -1) : current))
  }, [])

  const reset = useCallback(() => setStack([rootPage]), [rootPage])

  return {
    page,
    depth: stack.length,
    canGoBack: stack.length > 1,
    push,
    back,
    reset,
  }
}

interface SheetPageProps {
  title: ReactNode
  description?: ReactNode
  onBack?: () => void
  backLabel?: string
  children: ReactNode
  className?: string
  contentClassName?: string
}

/** One navigable page inside an existing BottomSheetContent. */
export function SheetPage({
  title,
  description,
  onBack,
  backLabel = 'Back',
  children,
  className,
  contentClassName,
}: SheetPageProps) {
  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <div className="flex shrink-0 items-start gap-2 px-4 pb-4 pr-14">
        {onBack ? (
          <button
            type="button"
            onClick={onBack}
            aria-label={backLabel}
            className="relative flex size-8 shrink-0 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
          >
            <span
              className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
            <ArrowLeft className="size-4 shrink-0" aria-hidden />
          </button>
        ) : null}
        <div className={cn('min-w-0 flex-1', onBack && 'pt-0.5')}>
          <BottomSheetTitle>{title}</BottomSheetTitle>
          {description ? (
            <BottomSheetDescription>{description}</BottomSheetDescription>
          ) : null}
        </div>
      </div>
      <div
        className={cn(
          'min-h-0 flex-1 overflow-y-auto overscroll-contain',
          contentClassName,
        )}
      >
        {children}
      </div>
    </div>
  )
}

interface SheetMenuGroupProps {
  children: ReactNode
  className?: string
}

export function SheetMenuGroup({ children, className }: SheetMenuGroupProps) {
  return (
    <div
      className={cn(
        'divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge',
        className,
      )}
    >
      {children}
    </div>
  )
}

interface SheetMenuRowProps {
  label: ReactNode
  value?: ReactNode
  icon?: ReactNode
  onClick: () => void
  disabled?: boolean
  showChevron?: boolean
  className?: string
}

export function SheetMenuRow({
  label,
  value,
  icon,
  onClick,
  disabled,
  showChevron = true,
  className,
}: SheetMenuRowProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'flex min-h-14 w-full min-w-0 items-center gap-3 px-3 text-left font-sans text-base text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus disabled:pointer-events-none disabled:opacity-40',
        className,
      )}
    >
      {icon ? <span className="shrink-0 text-ink-faint">{icon}</span> : null}
      <span className="min-w-0 flex-1 font-medium">{label}</span>
      {value ? (
        <span className="min-w-0 max-w-[55%] truncate text-ink-faint">
          {value}
        </span>
      ) : null}
      {showChevron ? (
        <ChevronRight className="size-4 shrink-0 text-ink-faint" aria-hidden />
      ) : null}
    </button>
  )
}

export interface SheetOption<Value extends string> {
  value: Value
  label: ReactNode
  description?: ReactNode
  icon?: ReactNode
  disabled?: boolean
}

interface SheetOptionListProps<Value extends string> {
  value: Value
  options: readonly SheetOption<Value>[]
  onChange: (value: Value) => void
  disabled?: boolean
  className?: string
}

/** Reusable radio-style option list for a drill-in sheet page. */
export function SheetOptionList<Value extends string>({
  value,
  options,
  onChange,
  disabled,
  className,
}: SheetOptionListProps<Value>) {
  const name = useId()
  return (
    <div
      className={cn(
        'divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge',
        className,
      )}
    >
      {options.map((option) => {
        const selected = option.value === value
        return (
          <label
            key={option.value}
            className={cn(
              'relative flex min-h-14 w-full min-w-0 cursor-pointer items-center gap-3 px-3 py-2 text-left font-sans text-base text-ink hover:bg-surface-hover has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-inset has-[:focus-visible]:ring-rule-focus',
              (disabled || option.disabled) &&
                'pointer-events-none cursor-default opacity-40',
            )}
          >
            <input
              type="radio"
              name={name}
              value={option.value}
              checked={selected}
              disabled={disabled || option.disabled}
              onChange={() => onChange(option.value)}
              className="sr-only"
            />
            {option.icon ? (
              <span className="shrink-0 text-ink-faint">{option.icon}</span>
            ) : null}
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="font-medium">{option.label}</span>
              {option.description ? (
                <span className="text-base leading-relaxed text-ink-faint">
                  {option.description}
                </span>
              ) : null}
            </span>
            {selected ? (
              <Check className="size-5 shrink-0 text-ink" aria-hidden />
            ) : null}
          </label>
        )
      })}
    </div>
  )
}
