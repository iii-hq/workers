import { ChevronRight, Folder, RadioTower } from 'lucide-react'
import { TriggerIcon } from '@/components/ui/TriggerIcon'

export type TimelineActivityKind =
  | 'function'
  | 'trigger-registration'
  | 'trigger'
  | 'working-dir'

interface TimelineActivityTrailProps {
  kind: TimelineActivityKind
}

/**
 * Persistent activity-kind marker shown beside the row's status icon. One
 * glyph and one color per kind — ƒ in accent for a function call, the bolt in
 * warn for a trigger fire, the tower in ok for a registration, the folder in
 * workdir (sky) for a session scope change — so a reader can tell the rows
 * apart from the left edge alone.
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
      ) : kind === 'trigger-registration' ? (
        <RadioTower aria-hidden="true" className="size-4 shrink-0 stroke-ok" />
      ) : kind === 'working-dir' ? (
        <Folder aria-hidden="true" className="size-4 shrink-0 stroke-workdir" />
      ) : (
        <TriggerIcon className="size-4 shrink-0 fill-warn" />
      )}
    </div>
  )
}

/** Hover/focus disclosure chevron kept at the trailing edge of the row. */
export function TimelineActivityDisclosure() {
  return (
    <div
      aria-hidden="true"
      className="relative flex w-8 shrink-0 items-center justify-center"
    >
      <ChevronRight className="size-4 shrink-0 stroke-ink-ghost opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100 group-hover/fchdr:opacity-100 group-focus-within/fchdr:opacity-100" />
    </div>
  )
}
