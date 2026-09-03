/**
 * The voice page's "Dictation test" panel: drives the shared dictation
 * controller with a large Start/Stop control and a read-only live transcript
 * textarea, so dictation works even when no chat composer is open.
 */

import { Button } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import type { DictationController } from '../lib/dictation'
import { useDictation } from '../lib/dictation'

export function DictationSection({
  controller,
  autoStartSignal,
}: {
  controller: DictationController
  autoStartSignal: number
}) {
  const { state, start, stop } = useDictation(controller)
  const appliedSignalRef = useRef(0)

  useEffect(() => {
    if (autoStartSignal > appliedSignalRef.current) {
      appliedSignalRef.current = autoStartSignal
      start()
    }
  }, [autoStartSignal, start])

  const listening = state.status === 'listening' || state.status === 'starting'
  const transcript = [...state.committed, state.partial].filter(Boolean).join(' ')

  return (
    <section className="voice-section">
      <h3 className="voice-section-title">Dictation test</h3>
      <div className="voice-dictation-controls">
        <Button
          variant={listening ? 'primary' : 'ghost'}
          onClick={() => (listening ? stop() : start())}
          disabled={state.status === 'stopping'}
        >
          {listening ? 'Stop' : 'Start'}
        </Button>
        <span className="voice-dictation-status">{state.status}</span>
      </div>
      {state.status === 'error' && state.error ? <div className="voice-note warn">{state.error}</div> : null}
      <textarea
        className="voice-dictation-transcript"
        readOnly
        value={transcript}
        placeholder="dictated text appears here…"
        aria-label="live dictation transcript"
      />
    </section>
  )
}
