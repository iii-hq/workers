import uiClasses from '@iii-dev/console-ui/ui-classes'
import { Check, Minus } from 'lucide-react'
import * as React from 'react'
import { cn } from '@/lib/utils'

export interface CheckboxProps
  extends Omit<
    React.InputHTMLAttributes<HTMLInputElement>,
    'children' | 'className' | 'type'
  > {
  /** Classes apply to the label wrapper; native input props stay on the checkbox. */
  className?: string
  /** Text beside the box; otherwise pass `aria-label`. */
  label?: React.ReactNode
  indeterminate?: boolean
}

/**
 * A native checkbox with the shared 18 px box: `surface` fill, accent when
 * checked, `rule-focus` outline, `Minus` for indeterminate. Wrapping the
 * input in the label keeps the whole row clickable.
 */
export const Checkbox = React.forwardRef<HTMLInputElement, CheckboxProps>(
  ({ className, label, indeterminate, ...props }, ref) => {
    const inner = React.useRef<HTMLInputElement>(null)
    React.useImperativeHandle(ref, () => inner.current as HTMLInputElement)
    React.useEffect(() => {
      if (inner.current) inner.current.indeterminate = Boolean(indeterminate)
    }, [indeterminate])
    return (
      <label className={cn(uiClasses.checkbox, className)}>
        <span className={uiClasses.checkboxControl}>
          <input
            ref={inner}
            type="checkbox"
            className={uiClasses.checkboxInput}
            {...props}
          />
          <Check
            aria-hidden
            className={uiClasses.checkboxMark}
            data-mark="check"
          />
          <Minus
            aria-hidden
            className={uiClasses.checkboxMark}
            data-mark="dash"
          />
        </span>
        {label != null ? (
          <span className={uiClasses.checkboxLabel}>{label}</span>
        ) : null}
      </label>
    )
  },
)
Checkbox.displayName = 'Checkbox'
