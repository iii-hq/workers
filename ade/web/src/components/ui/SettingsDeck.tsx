import { ChevronLeft } from 'lucide-react'
import * as React from 'react'
import { cn } from '@/lib/utils'
import { Button } from './Button'

export interface SettingsDeckProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children' | 'title'> {
  /** Whether the detail level is visible. The overview is shown when false. */
  open: boolean
  /** Collection, summary, or empty state shown at the deck's root level. */
  overview: React.ReactNode
  /** Settings for the selected resource. */
  detail: React.ReactNode
  /** Accessible heading for the selected resource. */
  title: React.ReactNode
  /** Optional context shown below the detail heading. */
  description?: React.ReactNode
  /** Visible label for the back action. Defaults to “Back”. */
  backLabel?: React.ReactNode
  /** More specific screen-reader label for the back action. */
  backAriaLabel?: string
  onBack: () => void
  /**
   * Focus the detail heading after pushing a level. Disable when the consumer
   * owns a more specific deep-link focus target.
   */
  autoFocusDetail?: boolean
}

/**
 * One-level settings navigation for resource collections. It presents one
 * pane at a time at every width, moves focus into a pushed detail, and restores
 * focus to the originating overview control when the user goes back. If that
 * control was removed, `data-settings-deck-fallback` marks the preferred
 * surviving focus target before the deck falls back to the first control.
 */
export const SettingsDeck = React.forwardRef<HTMLDivElement, SettingsDeckProps>(
  (
    {
      open,
      overview,
      detail,
      title,
      description,
      backLabel = 'Back',
      backAriaLabel,
      onBack,
      autoFocusDetail = true,
      className,
      onClickCapture,
      onFocusCapture,
      ...props
    },
    forwardedRef,
  ) => {
    const titleId = React.useId()
    const descriptionId = React.useId()
    const titleRef = React.useRef<HTMLHeadingElement | null>(null)
    const overviewRef = React.useRef<HTMLDivElement | null>(null)
    const overviewFocusRef = React.useRef<HTMLElement | null>(null)
    const previousOpenRef = React.useRef(false)

    React.useEffect(() => {
      const wasOpen = previousOpenRef.current
      previousOpenRef.current = open
      if (open === wasOpen) return

      const frame = window.requestAnimationFrame(() => {
        if (open) {
          if (autoFocusDetail) titleRef.current?.focus({ preventScroll: true })
          return
        }
        const origin = overviewFocusRef.current
        const target = origin?.isConnected
          ? origin
          : (overviewRef.current?.querySelector<HTMLElement>(
              '[data-settings-deck-fallback]:not([disabled]):not([aria-disabled="true"])',
            ) ??
            overviewRef.current?.querySelector<HTMLElement>(
              'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"]):not([aria-disabled="true"])',
            ))
        target?.focus({ preventScroll: true })
      })
      return () => window.cancelAnimationFrame(frame)
    }, [autoFocusDetail, open])

    const handleFocusCapture: React.FocusEventHandler<HTMLDivElement> = (
      event,
    ) => {
      if (!open && event.target instanceof HTMLElement) {
        overviewFocusRef.current = event.target
      }
      onFocusCapture?.(event)
    }

    const handleClickCapture: React.MouseEventHandler<HTMLDivElement> = (
      event,
    ) => {
      if (!open && event.target instanceof Element) {
        const target = event.target.closest<HTMLElement>(
          'button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        )
        if (target) overviewFocusRef.current = target
      }
      onClickCapture?.(event)
    }

    return (
      <div
        ref={forwardedRef}
        data-state={open ? 'detail' : 'overview'}
        className={cn('@container min-w-0', className)}
        onClickCapture={handleClickCapture}
        onFocusCapture={handleFocusCapture}
        {...props}
      >
        <div
          ref={overviewRef}
          hidden={open}
          className="min-w-0"
          data-settings-deck-pane="overview"
        >
          {overview}
        </div>

        <section
          hidden={!open}
          aria-labelledby={titleId}
          aria-describedby={description ? descriptionId : undefined}
          className="min-w-0"
          data-settings-deck-pane="detail"
        >
          <header className="mb-6 flex min-w-0 items-start gap-2 border-edge border-b pb-4">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="-ml-2 min-h-11 shrink-0 px-2 @lg:min-h-8"
              aria-label={backAriaLabel}
              onClick={onBack}
            >
              <ChevronLeft aria-hidden />
              {backLabel}
            </Button>
            <div className="min-w-0 flex-1 pt-1.5 sm:pt-1">
              <h2
                ref={titleRef}
                id={titleId}
                tabIndex={-1}
                className="m-0 truncate font-sans font-semibold text-base text-ink leading-6 outline-none sm:text-sm sm:leading-5"
              >
                {title}
              </h2>
              {description ? (
                <p
                  id={descriptionId}
                  className="mt-0.5 mb-0 font-sans text-ink-faint text-sm leading-5 sm:text-xs"
                >
                  {description}
                </p>
              ) : null}
            </div>
          </header>
          <div className="min-w-0">{detail}</div>
        </section>
      </div>
    )
  },
)
SettingsDeck.displayName = 'SettingsDeck'
