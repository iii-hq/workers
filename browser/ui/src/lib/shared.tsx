import { Chip } from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import type { ReactNode } from 'react'

/** A `label value` chip for a function-trigger card's request options. */
export function FilterChip({ label, value }: { label: string; value: ReactNode }) {
  return (
    <Chip>
      <span className={uiClasses.eyebrow}>{label}</span>
      <span>{value}</span>
    </Chip>
  )
}
