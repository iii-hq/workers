import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { Checkbox as BaseCheckbox } from '@/components/shadcn/checkbox'
import { cn } from '@/lib/utils'
import { changeEventFor } from './change-bridge'

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
 * The shadcn (Base UI) checkbox under the first console API: a wrapping
 * label keeps the whole row clickable, and `onChange` still receives a
 * change event whose `currentTarget.checked` is the new state (the
 * `change-bridge`), so worker UIs need no change.
 */
export const Checkbox = React.forwardRef<HTMLInputElement, CheckboxProps>(
  (
    {
      className,
      label,
      indeterminate,
      checked,
      defaultChecked,
      disabled,
      required,
      readOnly,
      name,
      value,
      id,
      onChange,
      ...rest
    },
    ref,
  ) => {
    const input = React.useRef<HTMLInputElement | null>(null)
    React.useImperativeHandle(ref, () => input.current as HTMLInputElement)
    return (
      // biome-ignore lint/a11y/noLabelWithoutControl: the Base UI checkbox renders its own hidden <input> inside this label
      <label className={cn(uiClasses.checkbox, className)}>
        <BaseCheckbox
          {...(rest as Record<string, unknown>)}
          id={id}
          name={name}
          value={typeof value === 'string' ? value : undefined}
          checked={checked}
          defaultChecked={defaultChecked}
          indeterminate={indeterminate}
          disabled={disabled}
          required={required}
          readOnly={readOnly}
          inputRef={input}
          className="shrink-0"
          onCheckedChange={(next, details) =>
            onChange?.(changeEventFor(input.current, next, details.event))
          }
        />
        {label != null ? (
          <span className={uiClasses.checkboxLabel}>{label}</span>
        ) : null}
      </label>
    )
  },
)
Checkbox.displayName = 'Checkbox'
