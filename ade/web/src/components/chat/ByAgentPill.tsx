import { shortAge } from '@/lib/short-age'
import { cn } from '@/lib/utils'

/**
 * "by agent · 2m": the mono, Faint pill after a description the agent wrote
 * (chat-projects §1.3.4). Without a time it reads "by agent".
 */
export function ByAgentPill({
  atMs,
  now,
  className,
}: {
  atMs?: number
  now?: number
  className?: string
}) {
  return (
    <span
      data-by-agent=""
      // The chat surface sets its font-mono in Inter; this pill is machine
      // text in Geist Mono (§1.3.4), in the header and in Conversation details.
      data-keep-mono=""
      className={cn(
        'inline-flex h-[18px] shrink-0 items-center rounded-sm bg-surface px-1.5 font-mono text-meta whitespace-nowrap text-ink-faint tabular-nums',
        className,
      )}
    >
      {atMs ? `by agent · ${shortAge(atMs, now)}` : 'by agent'}
    </span>
  )
}
