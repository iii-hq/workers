import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { cn } from '@/lib/utils'

interface CollapsibleCardContextValue {
  contentId: string
  disabled: boolean
  open: boolean
  setOpen(open: boolean): void
  triggerId: string
}

const CollapsibleCardContext = React.createContext<
  CollapsibleCardContextValue | undefined
>(undefined)

function useCollapsibleCard(part: string): CollapsibleCardContextValue {
  const context = React.useContext(CollapsibleCardContext)
  if (!context) {
    throw new Error(`${part} must be used inside CollapsibleCard`)
  }
  return context
}

export interface CollapsibleCardProps
  extends React.HTMLAttributes<HTMLDivElement> {
  /** Controlled expanded state. */
  open?: boolean
  /** Initial expanded state when the component is uncontrolled. */
  defaultOpen?: boolean
  /** Called after the trigger requests an expanded-state change. */
  onOpenChange?(open: boolean): void
  /** Prevent the trigger from changing the expanded state. */
  disabled?: boolean
}

/**
 * Shared card disclosure with accessible controlled/uncontrolled state and an
 * auto-height expand/collapse transition. Content stays mounted so worker UI
 * state survives while the card is collapsed.
 */
export const CollapsibleCard = React.forwardRef<
  HTMLDivElement,
  CollapsibleCardProps
>(
  (
    {
      children,
      className,
      defaultOpen = false,
      disabled = false,
      onOpenChange,
      open: controlledOpen,
      ...props
    },
    ref,
  ) => {
    const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen)
    const open = controlledOpen ?? uncontrolledOpen
    const reactId = React.useId()
    const triggerId = `${reactId}-trigger`
    const contentId = `${reactId}-content`

    const setOpen = React.useCallback(
      (nextOpen: boolean) => {
        if (controlledOpen === undefined) setUncontrolledOpen(nextOpen)
        onOpenChange?.(nextOpen)
      },
      [controlledOpen, onOpenChange],
    )

    const context = React.useMemo<CollapsibleCardContextValue>(
      () => ({ contentId, disabled, open, setOpen, triggerId }),
      [contentId, disabled, open, setOpen, triggerId],
    )

    return (
      <CollapsibleCardContext.Provider value={context}>
        <div
          ref={ref}
          {...props}
          className={cn(uiClasses.card, uiClasses.collapsibleCard, className)}
          data-disabled={disabled || undefined}
          data-state={open ? 'open' : 'closed'}
        >
          {children}
        </div>
      </CollapsibleCardContext.Provider>
    )
  },
)
CollapsibleCard.displayName = 'CollapsibleCard'

export type CollapsibleCardTriggerProps =
  React.ButtonHTMLAttributes<HTMLButtonElement>

/** The accessible button that toggles its enclosing CollapsibleCard. */
export const CollapsibleCardTrigger = React.forwardRef<
  HTMLButtonElement,
  CollapsibleCardTriggerProps
>(({ className, disabled, onClick, type = 'button', ...props }, ref) => {
  const context = useCollapsibleCard('CollapsibleCardTrigger')
  const isDisabled = context.disabled || disabled

  return (
    <button
      ref={ref}
      {...props}
      id={context.triggerId}
      type={type}
      aria-controls={context.contentId}
      aria-expanded={context.open}
      className={cn(uiClasses.collapsibleCardTrigger, className)}
      data-state={context.open ? 'open' : 'closed'}
      disabled={isDisabled}
      onClick={(event) => {
        onClick?.(event)
        if (!event.defaultPrevented && !isDisabled) {
          context.setOpen(!context.open)
        }
      }}
    />
  )
})
CollapsibleCardTrigger.displayName = 'CollapsibleCardTrigger'

export type CollapsibleCardContentProps = React.HTMLAttributes<HTMLElement>

/** Animated region controlled by CollapsibleCardTrigger. */
export const CollapsibleCardContent = React.forwardRef<
  HTMLElement,
  CollapsibleCardContentProps
>(({ children, className, ...props }, ref) => {
  const context = useCollapsibleCard('CollapsibleCardContent')

  return (
    <section
      ref={ref}
      {...props}
      id={context.contentId}
      aria-hidden={!context.open}
      aria-labelledby={context.triggerId}
      className={cn(uiClasses.collapsibleCardContent, className)}
      data-state={context.open ? 'open' : 'closed'}
      inert={context.open ? undefined : true}
    >
      <div className={uiClasses.collapsibleCardContentInner}>{children}</div>
    </section>
  )
})
CollapsibleCardContent.displayName = 'CollapsibleCardContent'
