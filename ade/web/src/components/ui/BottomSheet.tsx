import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import * as React from 'react'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'

export const BottomSheet = DialogPrimitive.Root
export const BottomSheetTrigger = DialogPrimitive.Trigger
export const BottomSheetClose = DialogPrimitive.Close

interface BottomSheetPageNavigationValue {
  activePageId: string | null
  renderedPageId: string | null
  pageHost: HTMLDivElement | null
  openPage: (id: string, labelledBy: string, describedBy?: string) => void
  closePage: (id: string) => void
}

const BottomSheetPageNavigationContext =
  React.createContext<BottomSheetPageNavigationValue | null>(null)

/**
 * Lets adaptive controls replace the current sheet content with an in-sheet
 * detail page instead of stacking a second dialog and overlay on top.
 */
export function useBottomSheetPageNavigation() {
  return React.useContext(BottomSheetPageNavigationContext)
}

interface BottomSheetContentProps
  extends React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content> {
  /** Optional visible heading for simple, single-page sheets. */
  heading?: React.ReactNode
  description?: React.ReactNode
  closeLabel?: string
  headerClassName?: string
  overlayClassName?: string
}

export const BottomSheetContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  BottomSheetContentProps
>(
  (
    {
      className,
      children,
      heading,
      description,
      closeLabel = 'Close',
      headerClassName,
      overlayClassName,
      onOpenAutoFocus,
      onCloseAutoFocus,
      'aria-labelledby': ariaLabelledBy,
      'aria-describedby': ariaDescribedBy,
      ...props
    },
    ref,
  ) => {
    const closeButtonRef = React.useRef<HTMLButtonElement>(null)
    const returnFocusRef = React.useRef<HTMLElement | null>(null)
    const pageActivationFrameRef = React.useRef<number | null>(null)
    const [activePageId, setActivePageId] = React.useState<string | null>(null)
    const [renderedPageId, setRenderedPageId] = React.useState<string | null>(
      null,
    )
    const [pageLabelledBy, setPageLabelledBy] = React.useState<string | null>(
      null,
    )
    const [pageDescribedBy, setPageDescribedBy] = React.useState<string | null>(
      null,
    )
    const [pageHost, setPageHost] = React.useState<HTMLDivElement | null>(null)
    const cancelPageActivation = React.useCallback(() => {
      if (
        pageActivationFrameRef.current !== null &&
        typeof window !== 'undefined'
      ) {
        window.cancelAnimationFrame(pageActivationFrameRef.current)
      }
      pageActivationFrameRef.current = null
    }, [])
    const openPage = React.useCallback(
      (id: string, labelledBy: string, describedBy?: string) => {
        cancelPageActivation()
        setRenderedPageId(id)
        setPageLabelledBy(labelledBy)
        setPageDescribedBy(describedBy ?? null)

        /* Page side-by-side needs the incoming page to reach its inactive
           offset for one paint before data-active flips. Mounting and
           activating in the same commit skips the transition entirely. */
        setActivePageId(null)
        if (
          typeof window === 'undefined' ||
          typeof window.requestAnimationFrame !== 'function' ||
          (typeof window.matchMedia === 'function' &&
            window.matchMedia('(prefers-reduced-motion: reduce)').matches)
        ) {
          setActivePageId(id)
          return
        }
        pageActivationFrameRef.current = window.requestAnimationFrame(() => {
          pageActivationFrameRef.current = window.requestAnimationFrame(() => {
            pageActivationFrameRef.current = null
            setActivePageId(id)
          })
        })
      },
      [cancelPageActivation],
    )
    const closePage = React.useCallback(
      (id: string) => {
        cancelPageActivation()
        setActivePageId((current) => (current === id ? null : current))
      },
      [cancelPageActivation],
    )
    React.useEffect(
      () => () => {
        cancelPageActivation()
      },
      [cancelPageActivation],
    )
    const pageNavigation = React.useMemo(
      () => ({
        activePageId,
        renderedPageId,
        pageHost,
        openPage,
        closePage,
      }),
      [activePageId, renderedPageId, pageHost, openPage, closePage],
    )
    const pageAria =
      activePageId !== null
        ? {
            'aria-labelledby': pageLabelledBy ?? undefined,
            'aria-describedby': pageDescribedBy ?? undefined,
          }
        : {
            ...(ariaLabelledBy !== undefined
              ? { 'aria-labelledby': ariaLabelledBy }
              : {}),
            ...(ariaDescribedBy !== undefined
              ? { 'aria-describedby': ariaDescribedBy }
              : {}),
          }

    return (
      <DialogPrimitive.Portal>
        <PortalScope>
          <DialogPrimitive.Overlay
            className={cn(
              'iii-ui-motion-sheet-overlay fixed inset-0 z-40 bg-black/55 md:hidden',
              overlayClassName,
            )}
          />
          <DialogPrimitive.Content
            ref={ref}
            className={cn(
              'iii-ui-motion-sheet fixed inset-x-3 bottom-3 z-50 flex max-h-[calc(100dvh-1.5rem)] flex-col overflow-hidden overscroll-contain md:hidden',
              'rounded-lg border border-edge bg-panel-raised text-ink shadow-floating',
              'pb-[max(0.75rem,env(safe-area-inset-bottom))] focus:outline-none',
              className,
            )}
            onOpenAutoFocus={(event) => {
              returnFocusRef.current =
                document.activeElement instanceof HTMLElement
                  ? document.activeElement
                  : null
              onOpenAutoFocus?.(event)
              if (event.defaultPrevented) return
              event.preventDefault()
              closeButtonRef.current?.focus({ preventScroll: true })
            }}
            onCloseAutoFocus={(event) => {
              onCloseAutoFocus?.(event)
              if (event.defaultPrevented || !returnFocusRef.current) return
              event.preventDefault()
              returnFocusRef.current.focus({ preventScroll: true })
            }}
            {...pageAria}
            {...props}
          >
            <div
              className="flex h-6 shrink-0 items-center justify-center"
              aria-hidden
            >
              <span className="h-1 w-10 rounded-full bg-ink-ghost/60" />
            </div>

            <BottomSheetPageNavigationContext.Provider value={pageNavigation}>
              <div className="relative flex min-h-0 flex-1 overflow-hidden">
                <div
                  data-active={activePageId === null}
                  aria-hidden={activePageId !== null}
                  inert={activePageId !== null}
                  className="iii-ui-motion-picker-page relative flex min-h-0 flex-1 flex-col [--picker-page-offset:calc(var(--distance-base)*-1)]"
                >
                  {heading ? (
                    <div
                      className={cn(
                        'shrink-0 px-4 pb-4 pr-14',
                        headerClassName,
                      )}
                    >
                      <BottomSheetTitle>{heading}</BottomSheetTitle>
                      {description ? (
                        <BottomSheetDescription>
                          {description}
                        </BottomSheetDescription>
                      ) : null}
                    </div>
                  ) : null}
                  {children}
                </div>

                <div
                  ref={setPageHost}
                  data-active={activePageId !== null}
                  aria-hidden={activePageId === null}
                  inert={activePageId === null}
                  className="iii-ui-motion-picker-page absolute inset-0 flex min-h-0 flex-col [--picker-page-offset:var(--distance-base)]"
                />
              </div>
            </BottomSheetPageNavigationContext.Provider>

            <DialogPrimitive.Close
              ref={closeButtonRef}
              className="absolute right-2 top-2 flex size-12 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
            >
              <X className="size-5" aria-hidden />
              <span className="sr-only">{closeLabel}</span>
            </DialogPrimitive.Close>
          </DialogPrimitive.Content>
        </PortalScope>
      </DialogPrimitive.Portal>
    )
  },
)
BottomSheetContent.displayName = 'BottomSheetContent'

export const BottomSheetTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn('font-sans text-lg font-semibold text-ink', className)}
    {...props}
  />
))
BottomSheetTitle.displayName = 'BottomSheetTitle'

export const BottomSheetDescription = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn(
      'font-sans text-base leading-relaxed text-ink-faint',
      className,
    )}
    {...props}
  />
))
BottomSheetDescription.displayName = 'BottomSheetDescription'
