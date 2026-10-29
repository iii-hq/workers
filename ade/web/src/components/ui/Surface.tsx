import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { Card as ShadcnCard } from '@/components/shadcn/card'
import { cn } from '@/lib/utils'

export interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  selected?: boolean
  interactive?: boolean
}

/**
 * The shadcn Card at its `sm` size. `.iii-ui-card` carries the same recipe
 * as plain CSS for worker UIs that only have class names; here it adds the
 * `selected` / `interactive` states. The legacy anatomy pads its own header
 * and body, so the card itself lays out as a block with no padding or gap.
 */
export const Card = React.forwardRef<HTMLDivElement, CardProps>(
  ({ className, selected, interactive, ...props }, ref) => (
    <ShadcnCard
      ref={ref}
      size="sm"
      data-selected={selected || undefined}
      data-interactive={interactive || undefined}
      className={cn(uiClasses.card, 'block gap-0 py-0', className)}
      {...props}
    />
  ),
)
Card.displayName = 'Card'

export const CardHeader = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    data-slot="card-header"
    className={cn(uiClasses.cardHeader, className)}
    {...props}
  />
))
CardHeader.displayName = 'CardHeader'

export const CardBody = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    data-slot="card-content"
    className={cn(uiClasses.cardBody, className)}
    {...props}
  />
))
CardBody.displayName = 'CardBody'

/** Borderless neutral inset for related content that needs emphasis inside a card. */
export const CardHighlight = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(uiClasses.cardHighlight, className)}
    {...props}
  />
))
CardHighlight.displayName = 'CardHighlight'

export const Panel = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    data-slot="card"
    className={cn(uiClasses.panel, className)}
    {...props}
  />
))
Panel.displayName = 'Panel'

export const PanelHeader = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn(uiClasses.panelHeader, className)} {...props} />
))
PanelHeader.displayName = 'PanelHeader'

export const PanelBody = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn(uiClasses.panelBody, className)} {...props} />
))
PanelBody.displayName = 'PanelBody'
