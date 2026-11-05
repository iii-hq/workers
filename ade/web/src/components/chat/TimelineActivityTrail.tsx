import { BookOpen, ChevronDown, Folder, RadioTower } from 'lucide-react'
import { ObserverIcon } from '@/components/ui/ObserverIcon'

export type TimelineActivityKind =
  | 'function'
  | 'observer-registration'
  | 'observer-fired'
  | 'working-dir'
  | 'skills'

interface TimelineActivityTrailProps {
  kind: TimelineActivityKind
}

/**
 * Persistent activity-kind marker shown beside the row's status icon. One
 * glyph and one color per kind — ƒ in accent for a function call, the bolt in
 * warn for an observer fire, the tower in ok for an observe, the folder in
 * workdir (sky) for a session scope change, the open book in quiet ink for a
 * skill-index refresh — so a reader can tell the rows apart from the left
 * edge alone.
 */
export function TimelineActivityTrail({ kind }: TimelineActivityTrailProps) {
  return (
    <div
      aria-hidden="true"
      className="flex size-4 shrink-0 items-center justify-center"
      data-timeline-activity-kind={kind}
    >
      {kind === 'function' ? (
        <div className="font-mono text-sm font-semibold text-accent italic">
          ƒ
        </div>
      ) : kind === 'observer-registration' ? (
        <RadioTower aria-hidden="true" className="size-4 shrink-0 stroke-ok" />
      ) : kind === 'working-dir' ? (
        <Folder aria-hidden="true" className="size-4 shrink-0 stroke-workdir" />
      ) : kind === 'skills' ? (
        <BookOpen
          aria-hidden="true"
          className="size-4 shrink-0 stroke-ink-faint"
        />
      ) : (
        <ObserverIcon className="size-4 shrink-0 fill-warn" />
      )}
    </div>
  )
}

/** Disclosure chevron at the trailing edge of a row; turns over when open. */
export function TimelineActivityDisclosure({ open }: { open?: boolean }) {
  return (
    <div
      aria-hidden="true"
      className="relative flex w-8 shrink-0 items-center justify-center"
    >
      <ChevronDown
        className="chat-tool-row__chevron size-4 shrink-0 stroke-ink-ghost"
        data-open={open ? 'true' : undefined}
      />
    </div>
  )
}
