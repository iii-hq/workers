import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { cn } from '@/lib/utils'

export interface SwitchProps
  extends Omit<
    React.InputHTMLAttributes<HTMLInputElement>,
    'children' | 'className' | 'type'
  > {
  /** Classes apply to the visual control; native input props stay on the checkbox. */
  className?: string
}

/**
 * A native checkbox presented as a switch. Checked, focus and disabled visuals
 * are driven entirely by CSS so browser form behavior remains intact.
 */
export const Switch = React.forwardRef<HTMLInputElement, SwitchProps>(
  ({ className, role = 'switch', ...props }, ref) => (
    <span className={cn(uiClasses.switch, className)}>
      <input
        ref={ref}
        type="checkbox"
        role={role}
        className={uiClasses.switchInput}
        {...props}
      />
      <span aria-hidden="true" className={uiClasses.switchThumb} />
    </span>
  ),
)
Switch.displayName = 'Switch'
