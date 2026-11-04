/**
 * A model's or engine's state as a quiet outlined pill with a tone dot:
 * the colour lives in the 6px dot, the words stay ink, so a row of states
 * reads at a glance without green or amber blocks.
 */

import { ShadcnBadge } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'

export type StatusTone = 'ok' | 'warn' | 'busy' | 'off'

export function StatusBadge({ tone, children }: { tone: StatusTone; children: ReactNode }) {
  return (
    <ShadcnBadge variant="outline" className="voice-status" data-tone={tone}>
      <span aria-hidden className="voice-status-dot" />
      {children}
    </ShadcnBadge>
  )
}
