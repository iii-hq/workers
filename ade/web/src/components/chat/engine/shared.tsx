import type { ReactNode } from 'react'
import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'

/**
 * Header row shared by all three engine list views. Renders a status pill
 * (count) on the left and a flexible chip row on the right for the active
 * request filters. Keeps the three list views visually consistent.
 */
interface ListHeaderProps {
  count: number
  noun: string
  filters?: ReactNode
  tone?: 'default' | 'accent' | 'warn'
}

export function ListHeader({
  count,
  noun,
  filters,
  tone = 'accent',
}: ListHeaderProps) {
  const label =
    count === 0
      ? `no ${noun} match`
      : `${count} ${count === 1 ? noun.replace(/s$/, '') : noun}`
  const pillVariant: 'default' | 'accent' | 'warn' =
    count === 0 ? 'warn' : tone === 'accent' ? 'accent' : tone
  return (
    <MetaRow>
      <StatusPill label={label} variant={pillVariant} />
      {filters ? (
        <span className="flex flex-wrap items-center gap-1.5">{filters}</span>
      ) : null}
    </MetaRow>
  )
}

interface FilterChipProps {
  label: string
  value: ReactNode
}

/** `LABEL value` chip. The label stays on the first line and the value wraps
 * anywhere, so a long id (scope, path) never leaves the label floating beside
 * a two-line block. */
export function FilterChip({ label, value }: FilterChipProps) {
  return (
    <Chip className="items-baseline">
      <span className="shrink-0 text-ink-faint uppercase tracking-[0.06em]">
        {label}
      </span>
      <span className="ml-1 min-w-0 wrap-anywhere text-ink">{value}</span>
    </Chip>
  )
}

export function InternalChip() {
  return (
    <Chip className="text-warn border-warn/40">
      <span>Internal</span>
    </Chip>
  )
}
