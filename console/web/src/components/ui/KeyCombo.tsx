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
}

export function KeyCombo({
  binding,
  platform = shortcutPlatform(),
  className,
  capClassName,
}: KeyComboProps) {
  const caps = formatBinding(binding, platform)
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
