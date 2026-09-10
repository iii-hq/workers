import { Download } from 'lucide-react'
import { useCallback, useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { downloadConversationAsMarkdown } from '@/lib/export-session'
import { downloadConversationWithSubagents } from '@/lib/export-session-full'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'

interface ExportSessionButtonProps {
  conversation: Conversation
  onExported?: (filename: string) => void
  className?: string
}

/**
 * Header-cluster dropdown that exports the current chat session as a
 * markdown file. Two options: "session only" (current chat) or "with
 * sub-agents" (includes sub-agent branching). Designed for the "paste
 * into another AI to analyse the skills" workflow — the file is
 * self-contained text, attachments are referenced by name only
 * (no base64 payload).
 *
 * Visual style mirrors the existing session-id copy button in `ChatView`,
 * with a 1200ms "exported" label flip after a successful download. While the
 * export is in flight the label shows "exporting…" and the trigger is disabled.
 */
export function ExportSessionButton({
  conversation,
  onExported,
  className,
}: ExportSessionButtonProps) {
  const [exported, setExported] = useState(false)
  const [busy, setBusy] = useState(false)
  const disabled = conversation.messages.length === 0

  const runExport = useCallback(
    async (download: (c: Conversation) => Promise<string>) => {
      if (disabled || busy) return
      setBusy(true)
      try {
        const filename = await download(conversation)
        setExported(true)
        window.setTimeout(() => setExported(false), 1200)
        onExported?.(filename)
      } catch {
        // Browsers without Blob/URL support are far outside our target;
        // swallow rather than crash the header.
      } finally {
        setBusy(false)
      }
    },
    [conversation, disabled, busy, onExported],
  )

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild disabled={disabled || busy}>
        <button
          type="button"
          disabled={disabled || busy}
          aria-label={busy ? 'exporting session' : 'export session'}
          className={cn(
            // `self-stretch`: the hit box is the full height of the header
            // group, not the 11px glyph box (WCAG 2.2 SC 2.5.8).
            'group relative flex self-stretch items-center gap-1 text-ink-faint hover:text-ink',
            'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
            disabled && 'opacity-50 cursor-not-allowed hover:text-ink-faint',
            className,
          )}
        >
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <Download className="size-4 shrink-0" aria-hidden />
          <span
            className={cn(
              'font-sans text-sm max-lg:hidden',
              exported && 'text-accent',
            )}
          >
            {busy ? 'exporting…' : exported ? 'exported' : 'export'}
          </span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem
          onSelect={() => void runExport(downloadConversationAsMarkdown)}
        >
          session only
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => void runExport(downloadConversationWithSubagents)}
        >
          with sub-agents
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
