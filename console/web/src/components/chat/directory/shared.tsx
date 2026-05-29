import type { ReactNode } from 'react'
import { Chip } from '@/components/chat/sandbox/shared'
import { Markdown } from '@/lib/markdown'

interface KvChipProps {
  label: string
  children: ReactNode
}

/** Two-tone chip with a small uppercase label and a value. Reused across
 * every directory view for filter + metadata badges. */
export function KvChip({ label, children }: KvChipProps) {
  return (
    <Chip>
      <span className="text-ink-faint uppercase tracking-[0.06em]">
        {label}
      </span>
      <span className="ml-1 text-ink">{children}</span>
    </Chip>
  )
}

/**
 * Render a markdown body (skill `.md`, prompt body, worker README) as actual
 * markdown — headings, fenced code, lists — via the same `<Markdown>` renderer
 * the chat transcript uses, rather than the raw monospace `<pre>` it used to be.
 */
export function MarkdownPane({
  body,
  className,
}: {
  body: string
  className?: string
}) {
  return (
    <div className={`px-3 py-2 bg-bg ${className ?? ''}`}>
      <Markdown>{body}</Markdown>
    </div>
  )
}

export function formatRelativeTime(value: string): string {
  if (!value) return ''
  const ts = Date.parse(value)
  if (Number.isNaN(ts)) return value
  const diffMs = Date.now() - ts
  if (diffMs < 0) return new Date(ts).toLocaleString()
  if (diffMs < 60_000) return `${Math.floor(diffMs / 1000)}s ago`
  if (diffMs < 3_600_000) return `${Math.floor(diffMs / 60_000)}m ago`
  if (diffMs < 86_400_000) return `${Math.floor(diffMs / 3_600_000)}h ago`
  if (diffMs < 7 * 86_400_000) return `${Math.floor(diffMs / 86_400_000)}d ago`
  return new Date(ts).toLocaleDateString()
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`
}
