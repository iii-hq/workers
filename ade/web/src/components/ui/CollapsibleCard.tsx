import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/shadcn/collapsible'
import { cn } from '@/lib/utils'

interface CollapsibleCardContextValue {
  contentId: string
  disabled: boolean
  open: boolean
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
 * A card that discloses: the shadcn (Base UI) collapsible in the card
 * recipe, with the first API's controlled / uncontrolled state. Content
 * stays mounted while collapsed, so worker UI state survives.
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
    const context = React.useMemo<CollapsibleCardContextValue>(
      () => ({ contentId, disabled, open, triggerId }),
      [contentId, disabled, open, triggerId],
    )
    return (
      <CollapsibleCardContext.Provider value={context}>
        <Collapsible
          ref={ref}
          open={open}
          disabled={disabled}
          onOpenChange={(next) => {
            if (controlledOpen === undefined) setUncontrolledOpen(next)
            onOpenChange?.(next)
          }}
          {...props}
          className={cn(uiClasses.card, uiClasses.collapsibleCard, className)}
          data-slot="card"
          data-disabled={disabled || undefined}
          data-state={open ? 'open' : 'closed'}
        >
          {children}
        </Collapsible>
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
>(({ className, type = 'button', ...props }, ref) => {
  const context = useCollapsibleCard('CollapsibleCardTrigger')
  return (
    <CollapsibleTrigger
      ref={ref}
      {...props}
      id={context.triggerId}
      type={type}
      aria-controls={context.contentId}
      className={cn(uiClasses.collapsibleCardTrigger, className)}
      data-state={context.open ? 'open' : 'closed'}
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
    <CollapsibleContent
      ref={ref as React.Ref<HTMLDivElement>}
      render={<section />}
      keepMounted
      {...props}
      id={context.contentId}
      aria-hidden={!context.open}
      aria-labelledby={context.triggerId}
      className={cn(uiClasses.collapsibleCardContent, className)}
      data-state={context.open ? 'open' : 'closed'}
      inert={context.open ? undefined : true}
    >
      <div className={uiClasses.collapsibleCardContentInner}>{children}</div>
    </CollapsibleContent>
  )
})
CollapsibleCardContent.displayName = 'CollapsibleCardContent'
