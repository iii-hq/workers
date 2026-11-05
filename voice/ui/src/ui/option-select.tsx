/**
 * A choice among labelled options, on the stock shadcn select: the trigger
 * shows the chosen label, the popup lists the options inside the select's
 * padded group (rounded rows inset from the edge, the check at the right),
 * each row its label over a quiet one-line description. Base UI reads `''`
 * as "nothing chosen", so an empty-string option (Let the router pick,
 * Choose a voice…) rides on a private sentinel.
 */

import {
  ShadcnSelect,
  ShadcnSelectContent,
  ShadcnSelectGroup,
  ShadcnSelectItem,
  ShadcnSelectTrigger,
  ShadcnSelectValue,
} from '@iii-dev/console-ui'

export interface Option {
  value: string
  label: string
  description?: string
}

const EMPTY = '\u0000empty'
const toInner = (value: string) => (value === '' ? EMPTY : value)
const toOuter = (value: string) => (value === EMPTY ? '' : value)

export function OptionSelect({
  value,
  options,
  onChange,
  disabled = false,
  id,
  'aria-label': ariaLabel,
  'aria-busy': ariaBusy,
}: {
  value: string
  options: readonly Option[]
  onChange: (next: string) => void
  disabled?: boolean
  id?: string
  'aria-label'?: string
  'aria-busy'?: boolean
}) {
  const items = options.map((option) => ({ value: toInner(option.value), label: option.label }))
  const described = options.some((option) => option.description)
  return (
    <ShadcnSelect
      items={items}
      value={toInner(value)}
      disabled={disabled}
      onValueChange={(next) => {
        if (next !== null && toOuter(next) !== value) onChange(toOuter(next))
      }}
    >
      <ShadcnSelectTrigger id={id} aria-label={ariaLabel} aria-busy={ariaBusy} className="voice-select">
        <ShadcnSelectValue />
      </ShadcnSelectTrigger>
      <ShadcnSelectContent
        align="start"
        alignItemWithTrigger={false}
        className="voice-select-content"
        data-described={described || undefined}
      >
        <ShadcnSelectGroup className="voice-select-group">
          {options.map((option) => (
            <ShadcnSelectItem
              key={option.value}
              value={toInner(option.value)}
              label={option.label}
              className="voice-select-item"
            >
              {option.description ? (
                <span className="voice-option">
                  <span className="voice-option-label">{option.label}</span>
                  <span className="voice-option-description">{option.description}</span>
                </span>
              ) : (
                option.label
              )}
            </ShadcnSelectItem>
          ))}
        </ShadcnSelectGroup>
      </ShadcnSelectContent>
    </ShadcnSelect>
  )
}
