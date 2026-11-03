import {
  ShadcnDropdownMenu,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuLabel,
  ShadcnDropdownMenuRadioGroup,
  ShadcnDropdownMenuRadioItem,
  ShadcnDropdownMenuTrigger,
} from '@iii-dev/console-ui'
import { type ReactElement, type ReactNode, type SyntheticEvent } from 'react'

export type MenuOption = {
  value: string
  label: string
  icon?: ReactNode
  /** Quiet text after the label (a count, "Archived"). */
  hint?: string
  disabled?: boolean
}

/** Clicks and keys stay inside: a card around the menu must not open when a row is picked (React bubbles through portals). */
export const swallow = (event: SyntheticEvent) => event.stopPropagation()

/**
 * Base UI renders a trigger through `render`, and the element's own props win the merge — children included.
 * So the trigger element keeps its children and the Trigger part gets none.
 */
export function splitTrigger(trigger: ReactElement): { render: ReactElement; children: ReactNode } {
  return { render: trigger, children: undefined }
}

/**
 * A short single-choice menu (status, priority, group by): a shadcn dropdown of radio rows with an icon each. The
 * trigger is any element, rendered through Base UI's `render` prop.
 */
export function OptionMenu({
  label,
  options,
  value,
  onChange,
  trigger,
  align = 'start',
  disabled,
}: {
  label: string
  options: MenuOption[]
  value: string
  onChange: (value: string) => void
  trigger: ReactElement
  align?: 'start' | 'center' | 'end'
  disabled?: boolean
}) {
  const { render, children } = splitTrigger(trigger)
  return (
    <ShadcnDropdownMenu>
      <ShadcnDropdownMenuTrigger disabled={disabled} onClick={swallow} onKeyDown={swallow} render={render}>
        {children}
      </ShadcnDropdownMenuTrigger>
      <ShadcnDropdownMenuContent align={align} className="kb-menu" onClick={swallow} onKeyDown={swallow}>
        <ShadcnDropdownMenuGroup>
          <ShadcnDropdownMenuLabel>{label}</ShadcnDropdownMenuLabel>
          <ShadcnDropdownMenuRadioGroup onValueChange={(next) => next !== value && onChange(next)} value={value}>
            {options.map((option) => (
              <ShadcnDropdownMenuRadioItem
                className="kb-menu__row"
                disabled={option.disabled}
                key={option.value}
                value={option.value}
              >
                {option.icon}
                <span className="kb-menu__label">{option.label}</span>
                {option.hint ? <span className="kb-menu__hint">{option.hint}</span> : null}
              </ShadcnDropdownMenuRadioItem>
            ))}
          </ShadcnDropdownMenuRadioGroup>
        </ShadcnDropdownMenuGroup>
      </ShadcnDropdownMenuContent>
    </ShadcnDropdownMenu>
  )
}
