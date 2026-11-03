import { ShadcnButton, ShadcnTooltip, ShadcnTooltipContent, ShadcnTooltipTrigger } from '@iii-dev/console-ui'
import type { MouseEvent, ReactNode } from 'react'

type IconActionProps = {
  /** Read by screen readers and shown as the tooltip. */
  label: string
  children: ReactNode
  onClick?: (event: MouseEvent<HTMLButtonElement>) => void
  variant?: 'ghost' | 'secondary' | 'default'
  size?: 'icon-xs' | 'icon-sm' | 'icon'
  disabled?: boolean
  className?: string
  /** Tooltip side; the label is always the accessible name. */
  side?: 'top' | 'bottom' | 'left' | 'right'
  /** For a disclosure toggle: whether what it controls is open. */
  expanded?: boolean
}

/** An icon-only shadcn button with its label as a tooltip — the board's one toolbar action. */
export function IconAction({
  label,
  children,
  onClick,
  variant = 'ghost',
  size = 'icon-sm',
  disabled,
  className,
  side = 'bottom',
  expanded,
}: IconActionProps) {
  return (
    <ShadcnTooltip>
      <ShadcnTooltipTrigger
        render={
          <ShadcnButton
            aria-expanded={expanded}
            aria-label={label}
            className={className}
            disabled={disabled}
            onClick={onClick}
            size={size}
            type="button"
            variant={variant}
          />
        }
      >
        {children}
      </ShadcnTooltipTrigger>
      <ShadcnTooltipContent side={side}>{label}</ShadcnTooltipContent>
    </ShadcnTooltip>
  )
}
