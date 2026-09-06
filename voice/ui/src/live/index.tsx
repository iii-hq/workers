/**
 * The `voice-live` turn summary: while a dictation runs, a row right above
 * the composer shows a pulsing dot and the words as they arrive. The slot
 * renders on every layout, so phones (where the chat header hides chips)
 * still see what the recognizer heard before it lands in the composer.
 */

import type { Host, SessionTurnSummaryProps, SessionTurnSummaryRegistration } from '@iii-dev/console-ui'
import type { DictationController } from '../lib/dictation'
import { useDictation } from '../lib/dictation'

const PARTIAL_MAX_CHARS = 160

export function createVoiceLiveSummary(_host: Host, controller: DictationController): SessionTurnSummaryRegistration {
  function VoiceLiveSummary(_props: SessionTurnSummaryProps) {
    const { state } = useDictation(controller)
    const listening = state.status === 'listening' || state.status === 'starting'
    if (!listening) return null
    const tail = [...state.committed.slice(-2), state.partial].filter(Boolean).join(' ')
    const shown = tail.length > PARTIAL_MAX_CHARS ? `…${tail.slice(-PARTIAL_MAX_CHARS)}` : tail
    return (
      <output className="voice-live" aria-live="polite">
        <span className="voice-chip-dot" aria-hidden="true" />
        <span className="voice-live-text">{shown || (state.status === 'starting' ? 'starting…' : 'listening')}</span>
      </output>
    )
  }

  return { id: 'voice-live', render: VoiceLiveSummary }
}
