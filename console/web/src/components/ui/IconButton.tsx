import * as React from 'react'
import { Button, type ButtonProps } from './Button'
import { Tooltip, TooltipContent, TooltipTrigger } from './Tooltip'

export interface IconButtonProps extends Omit<ButtonProps, 'size'> {
  label: string
  tooltip?: React.ReactNode | false
  tooltipSide?: 'top' | 'right' | 'bottom' | 'left'
}

/** Icon-only actions always own an accessible name and shared tooltip. */
export const IconButton = React.forwardRef<HTMLButtonElement, IconButtonProps>(
  (
    {
      label,
      tooltip = label,
      tooltipSide,
      variant = 'icon',
      children,
      ...props
    },
    ref,
  ) => {
    const button = (
      <Button
        ref={ref}
        type="button"
        variant={variant}
        size="icon"
        aria-label={props['aria-label'] ?? label}
        {...props}
      >
        {children}
      </Button>
    )

    if (tooltip === false) return button
    return (
      <Tooltip>
        <TooltipTrigger asChild>{button}</TooltipTrigger>
        <TooltipContent side={tooltipSide}>{tooltip}</TooltipContent>
      </Tooltip>
    )
  },
)
IconButton.displayName = 'IconButton'
