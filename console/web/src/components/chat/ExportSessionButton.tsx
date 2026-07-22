import { Download } from 'lucide-react'
import { useCallback, useState } from 'react'
import { downloadConversationAsMarkdown } from '@/lib/export-session'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'

interface ExportSessionButtonProps {
  conversation: Conversation
  onExported?: (filename: string) => void
  className?: string
}

/**
 * Header-cluster button that downloads the current chat session as a
 * markdown file. Designed for the "paste into another AI to analyse the
 * skills" workflow — the file is self-contained text, attachments are
 * referenced by name only (no base64 payload).
 *
 * Visual style mirrors the existing session-id copy button in
 * `ChatView` (`ChatView.tsx:517-537`): font-mono, 11px, uppercase,
 * `text-ink-faint hover:text-ink`, with a 1200ms "exported" label flip
 * after a successful download.
 */
export function ExportSessionButton({
  conversation,
  onExported,
  className,
}: ExportSessionButtonProps) {
  const [exported, setExported] = useState(false)
  const [busy, setBusy] = useState(false)
  const disabled = conversation.messages.length === 0

  const handleClick = useCallback(async () => {
    if (disabled || busy) return
    setBusy(true)
    try {
      const filename = await downloadConversationAsMarkdown(conversation)
      setExported(true)
      window.setTimeout(() => setExported(false), 1200)
      onExported?.(filename)
    } catch {
      // Browsers without Blob/URL support are far outside our target;
      // swallow rather than crash the header.
    } finally {
      setBusy(false)
    }
  }, [conversation, disabled, busy, onExported])

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={disabled}
      title={
        disabled
          ? 'no messages yet — nothing to export'
          : exported
            ? 'session downloaded'
            : 'download session as markdown — paste into another AI for analysis'
      }
      className={cn(
        'flex items-center gap-1 text-ink-faint hover:text-ink transition-colors group',
        'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
        disabled && 'opacity-50 cursor-not-allowed hover:text-ink-faint',
        className,
      )}
    >
      <Download className="size-3 flex-shrink-0" aria-hidden />
      <span
        className={cn(
          'text-[11px] uppercase tracking-[0.06em]',
          exported && 'text-accent',
        )}
      >
        {exported ? 'exported' : 'export'}
      </span>
    </button>
  )
}
