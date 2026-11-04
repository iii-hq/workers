import { Check, Copy, SquareTerminal } from 'lucide-react'
import type * as React from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import { Hint } from '@/components/ui/Hint'
import { copyTextToClipboard } from '@/lib/clipboard'
import { cn } from '@/lib/utils'

type CopyState = 'idle' | 'copied' | 'failed'

interface CommandCardProps {
  /** The command, verbatim — shown after the `$` prompt and copied. */
  command: string
  /** Header label beside the terminal glyph. */
  label?: string
  /** Output lines under the command (e.g. a live install log). */
  children?: React.ReactNode
  className?: string
}

/**
 * A shell command on a card: a header strip with the terminal glyph, a
 * label and a copy button, then the command in mono with any output
 * streamed beneath it.
 */
export function CommandCard({
  command,
  label = 'Terminal',
  children,
  className,
}: CommandCardProps) {
  const [copyState, setCopyState] = useState<CopyState>('idle')
  const flashRef = useRef<number | null>(null)

  useEffect(
    () => () => {
      if (flashRef.current != null) window.clearTimeout(flashRef.current)
    },
    [],
  )

  // The shared helper, never the raw async clipboard API: that API is
  // undefined over `http://<LAN-IP>` (insecure context) and would no-op.
  const handleCopy = useCallback(() => {
    void copyTextToClipboard(command).then((ok) => {
      setCopyState(ok ? 'copied' : 'failed')
      if (flashRef.current != null) window.clearTimeout(flashRef.current)
      flashRef.current = window.setTimeout(
        () => {
          flashRef.current = null
          setCopyState('idle')
          // A failure stays long enough to read and act on.
        },
        ok ? 1200 : 4000,
      )
    })
  }, [command])

  return (
    <div
      data-keep-mono
      className={cn(
        'overflow-hidden rounded-xl bg-card ring-1 ring-card-edge',
        className,
      )}
    >
      <div className="flex h-11 items-center gap-2.5 border-b border-card-edge pr-1.5 pl-3">
        <SquareTerminal
          className="size-4 shrink-0 text-ink-faint"
          aria-hidden
        />
        <span className="font-inter text-[13px] font-medium text-ink">
          {label}
        </span>
        {/* A failed copy says so in the card, not only in a tooltip (which
            touch never shows), so the reader knows to select it by hand. */}
        {copyState === 'failed' ? (
          <span className="ml-auto font-inter text-[13px] text-warn">
            Couldn't copy, select the command instead
          </span>
        ) : null}
        <span role="status" className="sr-only">
          {copyState === 'copied'
            ? 'Command copied'
            : copyState === 'failed'
              ? "Couldn't copy the command"
              : ''}
        </span>
        <Hint
          label={
            copyState === 'copied'
              ? 'Copied'
              : copyState === 'failed'
                ? "Couldn't copy"
                : 'Copy command'
          }
        >
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Copy command"
            className={cn(
              'text-ink-faint hover:text-ink',
              copyState !== 'failed' && 'ml-auto',
            )}
            onClick={handleCopy}
          >
            {copyState === 'copied' ? (
              <Check className="size-4 text-accent" aria-hidden />
            ) : (
              <Copy className="size-4" aria-hidden />
            )}
          </Button>
        </Hint>
      </div>
      <div className="px-4 py-3.5 font-mono text-[13px] leading-[1.6]">
        <div className="flex gap-2.5">
          <span aria-hidden className="select-none text-ink-ghost">
            $
          </span>
          <span className="min-w-0 break-all text-ink">{command}</span>
        </div>
        {children ? <div className="mt-2">{children}</div> : null}
      </div>
    </div>
  )
}
