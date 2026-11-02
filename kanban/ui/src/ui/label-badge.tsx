import { ShadcnBadge } from '@iii-dev/console-ui'
import { toneOf } from './tone'

/* Tailwind (`tw:`, see tailwind.mjs): an outlined pill, neutral-800 in dark, with the label's hue as a dot. */
const PILL =
  'kb-label tw:h-6 tw:max-w-40 tw:gap-1.5 tw:rounded-full tw:border tw:border-neutral-100 tw:bg-transparent tw:pr-2.5 tw:pl-2 tw:text-[13px] tw:font-medium tw:text-neutral-600 tw:dark:border-neutral-800 tw:dark:text-neutral-400'
const PILL_ON = 'tw:border-neutral-400 tw:text-neutral-900 tw:dark:border-neutral-600 tw:dark:text-neutral-100'

/** A label as an outline badge with a hue dot: the same word always wears the same colour. */
export function LabelBadge({ label, selected }: { label: string; selected?: boolean }) {
  return (
    <ShadcnBadge className={`${PILL} ${selected ? PILL_ON : ''}`} data-selected={selected ? 'true' : undefined} variant="outline">
      <span aria-hidden="true" className="tw:size-2 tw:shrink-0 tw:rounded-full tw:bg-(--kb-tone)" data-tone={toneOf(label)} />
      <span className="tw:truncate">{label}</span>
    </ShadcnBadge>
  )
}

/** Up to `max` labels, then `+N` with the rest in its title. */
export function LabelList({ labels, highlight = [], max = 3 }: { labels: string[]; highlight?: string[]; max?: number }) {
  if (labels.length === 0) return null
  const ordered = [...labels].sort((a, b) => Number(highlight.includes(b)) - Number(highlight.includes(a)))
  const rest = ordered.slice(max)
  return (
    <span className="kb-labels tw:inline-flex tw:min-w-0 tw:flex-wrap tw:gap-1.5">
      {ordered.slice(0, max).map((label) => (
        <LabelBadge key={label} label={label} selected={highlight.includes(label)} />
      ))}
      {rest.length > 0 ? (
        <ShadcnBadge className={`${PILL} tw:px-2`} title={rest.join(', ')} variant="outline">
          +{rest.length}
        </ShadcnBadge>
      ) : null}
    </span>
  )
}
