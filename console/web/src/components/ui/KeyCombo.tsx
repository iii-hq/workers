/**
 * A shortcut as key caps, spelled for the reader's keyboard.
 *
 * Mac renders the glyphs adjacent the way the system does (⌘K); everywhere
 * else the words need a separator to read as a chord (ctrl + k).
 */

import { Fragment } from 'react'
import {
  formatBinding,
  type Platform,
  shortcutPlatform,
} from '@/lib/keybindings/bindings'
import { cn } from '@/lib/utils'

interface KeyComboProps {
  binding: string
  platform?: Platform
  className?: string
  capClassName?: string
  /** Selects by position: the stored chord ends in a digit but the shortcut
   *  fires for any of 1 to 9, so the last cap shows the whole range. */
  digitRange?: boolean
}

export function KeyCombo({
  binding,
  platform = shortcutPlatform(),
  className,
  capClassName,
  digitRange = false,
}: KeyComboProps) {
  const formatted = formatBinding(binding, platform)
  const last = formatted.at(-1)
  const caps =
    digitRange && last !== undefined && /^[1-9]$/.test(last)
      ? [...formatted.slice(0, -1), `${last}–9`]
      : formatted
  const mac = platform === 'mac'
  return (
    <span className={cn('inline-flex items-center gap-1', className)}>
      {caps.map((cap, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a chord can repeat a cap and position is the identity
        <Fragment key={`${cap}-${index}`}>
          <kbd
            className={cn(
              'inline-flex min-w-5 items-center justify-center rounded border border-edge px-1 py-px font-mono text-[0.65rem] text-ink-ghost',
              capClassName,
            )}
          >
            {cap}
          </kbd>
          {!mac && index < caps.length - 1 ? (
            <span aria-hidden className="text-[0.65rem] text-ink-ghost">
              +
            </span>
          ) : null}
        </Fragment>
      ))}
    </span>
  )
}
