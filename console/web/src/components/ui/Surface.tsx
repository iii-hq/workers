import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { cn } from '@/lib/utils'

export interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  selected?: boolean
  interactive?: boolean
}

export const Card = React.forwardRef<HTMLDivElement, CardProps>(
  ({ className, selected, interactive, ...props }, ref) => (
    <div
      ref={ref}
      data-selected={selected || undefined}
      data-interactive={interactive || undefined}
      className={cn(uiClasses.card, className)}
      {...props}
    />
  ),
)
Card.displayName = 'Card'

export const CardHeader = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn(uiClasses.cardHeader, className)} {...props} />
))
CardHeader.displayName = 'CardHeader'

export const CardBody = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn(uiClasses.cardBody, className)} {...props} />
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
  <div ref={ref} className={cn(uiClasses.panel, className)} {...props} />
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
