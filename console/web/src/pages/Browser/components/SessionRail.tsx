import { Globe } from 'lucide-react'
import type { BrowserSessionInfo } from '@/lib/browser'
import { cn } from '@/lib/utils'

/**
 * Left rail: one row per live Chromium session. Selection routes to
 * `#/browser/<session_id>` so the viewport deep-links and survives reloads.
 */

interface SessionRailProps {
  sessions: BrowserSessionInfo[]
  selectedId: string | null
  onSelect: (sessionId: string) => void
}

function hostOf(url: string): string {
  try {
    const parsed = new URL(url)
    return parsed.host || url
  } catch {
    return url
  }
}

function relativeTime(ms: number): string {
  const delta = Date.now() - ms
  if (delta < 60_000) return 'now'
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`
  return `${Math.floor(delta / 86_400_000)}d ago`
}

export function SessionRail({
  sessions,
  selectedId,
  onSelect,
}: SessionRailProps) {
  return (
    <nav aria-label="browser sessions" className="flex flex-col">
      {sessions.map((session) => {
        const selected = session.session_id === selectedId
        return (
          <button
            key={session.session_id}
            type="button"
            onClick={() => onSelect(session.session_id)}
            aria-current={selected ? 'true' : undefined}
            title={`${session.session_id} · ${session.url}`}
            className={cn(
              'w-full text-left border-b border-rule-2 px-3 py-2.5 font-mono transition-colors',
              selected ? 'bg-panel' : 'hover:bg-paper-2',
            )}
          >
            <span className="flex items-center gap-1.5 min-w-0">
              <Globe
                size={12}
                aria-hidden
                className={cn(
                  'shrink-0',
                  selected ? 'text-accent' : 'text-ink-faint',
                )}
              />
              <span className="truncate text-[12px] text-ink">
                {session.title?.trim() || hostOf(session.url) || 'about:blank'}
              </span>
            </span>
            <span className="mt-0.5 flex items-center gap-2 text-[11px] text-ink-faint min-w-0">
              <span className="truncate min-w-0">{session.url}</span>
            </span>
            <span className="mt-0.5 flex items-center gap-2 text-[10px] lowercase text-ink-ghost">
              <span className="tabular-nums">{session.session_id}</span>
              <span>·</span>
              <span>{session.headless ? 'headless' : 'headful'}</span>
              <span>·</span>
              <span>{relativeTime(session.last_used_ms)}</span>
            </span>
          </button>
        )
      })}
    </nav>
  )
}
