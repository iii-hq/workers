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
    const settled = state.committed.slice(-2).join(' ')
    const room = Math.max(0, PARTIAL_MAX_CHARS - state.partial.length)
    const settledShown = settled.length > room ? `…${settled.slice(settled.length - room)}` : settled
    const idle = state.status === 'starting' ? 'starting…' : 'listening'
    return (
      <output className="voice-live" aria-live="polite">
        <span className="voice-chip-dot" aria-hidden="true" />
        <span className="voice-live-text">
          {settledShown || state.partial ? (
            <>
              {settledShown}
              {settledShown && state.partial ? ' ' : ''}
              {state.partial ? (
                <span
                  className="voice-live-partial"
                  title="Live words from the streaming model; the final text arrives when the sentence ends"
                >
                  {state.partial}
                </span>
              ) : null}
            </>
          ) : (
            idle
          )}
        </span>
      </output>
    )
  }

  return { id: 'voice-live', render: VoiceLiveSummary }
}
