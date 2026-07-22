import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import { copyTextToClipboard } from '@/lib/clipboard'
import { cn } from '@/lib/utils'

/**
 * Copy affordance for a chat message — the PaneShell copy idiom (12px
 * Copy→Check flip on success, nothing on failure) lifted to a message
 * header. Visibility is the parent's job (hover/focus opacity classes);
 * this component only copies and flips.
 */
export function CopyMessageButton({
  text,
  label = 'copy message',
  className,
}: {
  /** A thunk defers building large payloads (e.g. serialized function calls)
      until the click, keeping streaming re-renders free. */
  text: string | (() => string)
  /** Accessible name while idle (aria-label/title); flips to "copied". */
  label?: string
  className?: string
}) {
  const [copied, setCopied] = useState(false)
  const copy = () => {
    const value = typeof text === 'function' ? text() : text
    void copyTextToClipboard(value).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }
  return (
    <button
      type="button"
      onClick={copy}
      className={cn(
        'shrink-0 text-ink-ghost hover:text-ink transition-colors',
        className,
      )}
      aria-label={copied ? 'copied' : label}
      title={copied ? 'copied' : label}
    >
      {copied ? (
        <Check size={12} aria-hidden />
      ) : (
        <Copy size={12} aria-hidden />
      )}
    </button>
  )
}
