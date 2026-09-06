import { ChevronDown } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { PathGlyph } from './FileMentionNode'

/**
 * One-line rows for the composer's typeahead menus. The name carries the
 * weight (small, semibold), the detail — a function's description, a
 * file's folder — trails it in faint ink and is the part that gives way
 * to the ellipsis first. The row is a flex line with `min-w-0` on both
 * spans so `truncate` actually has something to truncate against.
 */

interface MentionRowProps {
  icon: ReactNode
  name: string
  detail?: string
  /** Emphasise the name less (a folder row, say). */
  quiet?: boolean
}

export function MentionRow({ icon, name, detail, quiet }: MentionRowProps) {
  return (
    <>
      <span
        aria-hidden="true"
        className="flex w-4 shrink-0 items-center justify-center text-accent"
      >
        {icon}
      </span>
      <span
        className={cn(
          'min-w-0 max-w-[70%] shrink truncate font-mono text-[12px] font-semibold text-ink',
          quiet && 'text-ink-faint',
        )}
      >
        {name}
      </span>
      {detail ? (
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-faint">
          {detail}
        </span>
      ) : null}
    </>
  )
}

/** The `ƒ` mark every function surface uses. */
export function FunctionGlyph() {
  return (
    <span className="font-semibold italic leading-none text-[13px]">ƒ</span>
  )
}

/** File or folder outline, the same one the inserted pill shows. */
export function FileGlyph({ path }: { path: string }) {
  return <PathGlyph path={path} />
}

/** The row that reveals the next page of a long list. */
export function MoreRow({ remaining }: { remaining: number }) {
  return (
    <>
      <span
        aria-hidden="true"
        className="flex w-4 shrink-0 items-center justify-center text-ink-faint"
      >
        <ChevronDown className="size-4" />
      </span>
      <span className="min-w-0 truncate font-mono text-[11px] font-semibold text-ink-faint">
        show more
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-ghost tabular-nums">
        {remaining} left
      </span>
    </>
  )
}
