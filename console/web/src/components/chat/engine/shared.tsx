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

export function FilterChip({ label, value }: FilterChipProps) {
  return (
    <Chip>
      <span className="text-ink-faint uppercase tracking-[0.06em]">
        {label}
      </span>
      <span className="ml-1 text-ink">{value}</span>
    </Chip>
  )
}

export function InternalChip() {
  return (
    <Chip className="text-warn border-warn/40">
      <span className="uppercase tracking-[0.06em]">internal</span>
    </Chip>
  )
}
