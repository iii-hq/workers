import { Copy } from 'lucide-react'
import type * as React from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Prompt } from '@/components/ui/Prompt'
import { copyTextToClipboard } from '@/lib/clipboard'
import { cn } from '@/lib/utils'

type CopyState = 'idle' | 'copied' | 'failed'

interface TerminalCommandLineProps {
  /** The command, verbatim — also the hover title and the copy payload. */
  command: string
  /** Prompt glyph, accent ink. */
  prompt?: string
  /** Trailing slot for header chips / pills. */
  chips?: React.ReactNode
  /** Render the copy affordance (flashes copied / failed). */
  copy?: boolean
  className?: string
}

/**
 * One-line command header — the `$ command` row a terminal-shaped card
 * opens with. The prompt glyph is accent ink, the command ellipsizes with
 * the full text riding on `title`, `chips` trail on the right, and `copy`
 * adds the standard copy affordance: the clipboard helper that survives
 * insecure origins, flashing the outcome either way.
 */
export function TerminalCommandLine({
  command,
  prompt = '$',
  chips,
  copy,
  className,
}: TerminalCommandLineProps) {
  const [copyState, setCopyState] = useState<CopyState>('idle')
  const flashRef = useRef<number | null>(null)

  // A pending flash reset must not fire into an unmounted component.
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
      flashRef.current = window.setTimeout(() => {
        flashRef.current = null
        setCopyState('idle')
      }, 1200)
    })
  }, [command])

  return (
    <div className={cn('flex min-w-0 items-center gap-2', className)}>
      <Prompt symbol={prompt} className="shrink-0 text-[12.5px]" />
      <span
        className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-ink"
        title={command}
      >
        {command}
      </span>
      {chips ? (
        <span className="flex shrink-0 items-center gap-1.5">{chips}</span>
      ) : null}
      {copy ? (
        <button
          type="button"
          onClick={handleCopy}
          title={
            copyState === 'idle'
              ? 'copy command'
              : copyState === 'copied'
                ? 'copied'
                : 'copy failed'
          }
          className={cn(
            'flex shrink-0 cursor-pointer items-center gap-1 text-ink-faint transition-colors hover:text-ink',
            'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
          )}
        >
          {copyState === 'idle' ? (
            <>
              <Copy className="size-3 shrink-0" aria-hidden />
              <span className="font-mono text-[10px] uppercase tracking-[0.06em]">
                copy
              </span>
            </>
          ) : (
            <span
              className={cn(
                'font-mono text-[10px] uppercase tracking-[0.06em]',
                copyState === 'copied' ? 'text-accent' : 'text-warn',
              )}
            >
              {copyState === 'copied' ? 'copied' : 'failed'}
            </span>
          )}
        </button>
      ) : null}
    </div>
  )
}
