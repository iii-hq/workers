import * as React from 'react'
import { Switch as BaseSwitch } from '@/components/shadcn/switch'
import { cn } from '@/lib/utils'
import { changeEventFor } from './change-bridge'

export interface SwitchProps
  extends Omit<
    React.InputHTMLAttributes<HTMLInputElement>,
    'children' | 'className' | 'type'
  > {
  /** Classes apply to the visual control; native input props stay on the switch. */
  className?: string
}

/**
 * The shadcn (Base UI) switch under the first console API: `onChange`
 * still receives a change event whose `currentTarget.checked` is the new
 * state (the `change-bridge`), so worker UIs need no change.
 */
export const Switch = React.forwardRef<HTMLInputElement, SwitchProps>(
  (
    {
      className,
      checked,
      defaultChecked,
      disabled,
      required,
      readOnly,
      name,
      value,
      id,
      onChange,
      role: _role,
      ...rest
    },
    ref,
  ) => {
    const input = React.useRef<HTMLInputElement | null>(null)
    React.useImperativeHandle(ref, () => input.current as HTMLInputElement)
    return (
      <BaseSwitch
        {...(rest as Record<string, unknown>)}
        id={id}
        name={name}
        value={typeof value === 'string' ? value : undefined}
        checked={checked}
        defaultChecked={defaultChecked}
        disabled={disabled}
        required={required}
        readOnly={readOnly}
        inputRef={input}
        className={cn(className)}
        onCheckedChange={(next, details) =>
          onChange?.(changeEventFor(input.current, next, details.event))
        }
      />
    )
  },
)
Switch.displayName = 'Switch'
