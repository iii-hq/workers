/**
 * A tooltip for an icon-only control, on the shadcn (Base UI) tooltip.
 *
 * `children` is the control itself; it keeps its own props and ref, the
 * tooltip only adds its hover and focus listeners. A `shortcut` names a
 * registry action and the chord is spelled beside the label as caps, so
 * the tooltip teaches the key as well as the name.
 */

import type { ReactElement, ReactNode } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/shadcn/tooltip'
import { formatBinding, shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  bindingsFor,
  type KeybindingActionId,
} from '@/lib/keybindings/registry'

export interface HintProps {
  label: ReactNode
  /** A registry action whose first chord is shown beside the label. */
  shortcut?: KeybindingActionId
  side?: 'top' | 'bottom' | 'left' | 'right'
  align?: 'start' | 'center' | 'end'
  /** Nothing to say right now (a control with its own visible label). */
  disabled?: boolean
  /** On the pill, e.g. a wider `max-w-*` for a long message. */
  className?: string
  children: ReactElement
}

/** The chord as the tooltip spells it: `⌘K` on a Mac, `Ctrl+K` elsewhere. */
export function hintShortcut(id: KeybindingActionId): string | null {
  const platform = shortcutPlatform()
  const binding = bindingsFor(id, platform)[0]
  if (!binding) return null
  const caps = formatBinding(binding, platform)
  return platform === 'mac' ? caps.join('') : caps.join('+')
}

export function Hint({
  label,
  shortcut,
  side = 'bottom',
  align = 'center',
  disabled = false,
  className,
  children,
}: HintProps) {
  if (disabled) return children
  const chord = shortcut ? hintShortcut(shortcut) : null
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent side={side} align={align} className={className}>
        {label}
        {chord ? (
          // The cap's look lives in the tooltip recipe (`data-slot=kbd`).
          <kbd data-slot="kbd" className="ml-0.5">
            {chord}
          </kbd>
        ) : null}
      </TooltipContent>
    </Tooltip>
  )
}
