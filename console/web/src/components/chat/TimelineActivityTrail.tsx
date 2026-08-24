import { ChevronRight } from 'lucide-react'
import { TriggerIcon } from '@/components/ui/TriggerIcon'

interface TimelineActivityTrailProps {
  kind: 'function' | 'trigger'
}

/** Persistent activity-kind marker shown beside the row's status icon. */
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
