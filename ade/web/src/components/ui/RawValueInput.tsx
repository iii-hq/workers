import * as React from 'react'
import { cn } from '@/lib/utils'
import { Button } from './Button'
import { Chip } from './Chip'
import { Input, type InputProps } from './Input'

export interface RawValueInputProps extends Omit<InputProps, 'className'> {
  /** Human label used by the explicit replacement action. */
  label: string
  kind: 'environment' | 'custom'
  replacementLabel: React.ReactNode
  onUseLiteral: () => void
  className?: string
  inputClassName?: string
}

/**
 * An opaque/template value editor with an explicit path back to a typed
 * literal. It never interprets or replaces the raw value by itself.
 */
export const RawValueInput = React.forwardRef<
  HTMLInputElement,
  RawValueInputProps
>(
  (
    {
      label,
      kind,
      replacementLabel,
      onUseLiteral,
      className,
      inputClassName,
      ...inputProps
    },
    ref,
  ) => (
    <div
      className={cn(
        'grid min-w-0 grid-cols-1 items-end gap-2 sm:grid-cols-[minmax(0,1fr)_auto]',
        className,
      )}
      data-raw-value-kind={kind}
    >
      <div className="flex min-w-0 flex-col gap-1.5">
        <Chip className="self-start" tone="warning">
          {kind === 'environment' ? 'Environment' : 'Custom value'}
        </Chip>
        <Input
          ref={ref}
          className={inputClassName}
          spellCheck={false}
          autoComplete="off"
          {...inputProps}
        />
      </div>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="min-h-11 px-3 sm:min-h-8"
        aria-label={`Replace ${label} raw value with a literal value`}
        onClick={onUseLiteral}
      >
        Use {replacementLabel}
      </Button>
    </div>
  ),
)
RawValueInput.displayName = 'RawValueInput'
