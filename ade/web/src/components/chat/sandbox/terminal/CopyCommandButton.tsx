import { Copy } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Eyebrow } from '@/components/ui/Eyebrow'
import { copyTextToClipboard } from '@/lib/clipboard'
import { cn } from '@/lib/utils'

interface CopyCommandButtonProps {
  text: string
  className?: string
}

/**
 * Ghost copy control for the `Terminal` header chip row. Visual style
 * mirrors the session-id copy button in `ChatView`.
 */
export function CopyCommandButton({ text, className }: CopyCommandButtonProps) {
  const [copied, setCopied] = useState(false)

  const handleCopy = useCallback(() => {
    // Helper, not navigator.clipboard: the API is undefined over
    // `http://<LAN-IP>` (insecure context) and the raw call no-ops.
    void copyTextToClipboard(text).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }, [text])

  return (
    <button
      type="button"
      onClick={handleCopy}
      title={copied ? `copied ${text}` : `copy ${text}`}
      className={cn(
        'flex items-center gap-1 text-ink-faint hover:text-ink transition-colors',
        'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
        className,
      )}
    >
      {copied ? (
        <Eyebrow className="text-accent">copied</Eyebrow>
      ) : (
        <>
          <Copy className="size-4 flex-shrink-0" aria-hidden />
          <Eyebrow className="text-current">copy</Eyebrow>
        </>
      )}
    </button>
  )
}
