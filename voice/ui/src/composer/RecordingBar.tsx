/**
 * What the composer's toolbar turns into while you dictate: the live
 * waveform across the row, the time spoken so far, a cross that throws the
 * recording away and a check that stops and puts the text in the message
 * box. It lies over the toolbar (the console makes the toolbar its
 * containing block) and goes once the dictation is over. While the mic is
 * held down for push-to-talk it lets the pointer through, so lifting the
 * finger still lands on the mic.
 */

import { IconButton } from '@iii-dev/console-ui'
import { Check, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { DictationController, DictationStatus } from '../lib/dictation'
import { formatClock } from '../lib/format'
import { LiveWaveform } from '../lib/live-waveform'

export function RecordingBar({
  controller,
  status,
  holding,
  onDone,
  onCancel,
}: {
  controller: DictationController
  status: DictationStatus
  holding: boolean
  onDone: () => void
  onCancel: () => void
}) {
  const elapsed = useListeningClock(status)
  const doneRef = useRef<HTMLButtonElement>(null)
  const stopping = status === 'stopping'

  // A click on the mic leaves focus on a control now under the bar: carry it to Done.
  useEffect(() => {
    const active = document.activeElement
    if (active instanceof HTMLElement && active.closest('.voice-composer-btn')) doneRef.current?.focus()
  }, [])

  return (
    <div className="voice-recording-bar" data-status={status} data-holding={holding || undefined} role="group" aria-label="Dictation">
      <LiveWaveform
        className="voice-recording-wave"
        active={status === 'listening'}
        processing={stopping}
        getAnalyser={controller.getAnalyser}
      />
      <span className="voice-recording-time" aria-label={`${Math.floor(elapsed)} seconds`}>
        {formatClock(elapsed)}
      </span>
      <IconButton label="Cancel dictation" tooltip="Discard" className="voice-recording-cancel" disabled={stopping} onClick={onCancel}>
        <X />
      </IconButton>
      <IconButton
        ref={doneRef}
        label="Finish dictation"
        tooltip={stopping ? 'Finishing…' : 'Insert the text'}
        className={['voice-recording-done', stopping ? 'is-stopping' : ''].filter(Boolean).join(' ')}
        aria-busy={stopping || undefined}
        onClick={() => {
          if (!stopping) onDone()
        }}
      >
        <Check />
      </IconButton>
    </div>
  )
}

/** Seconds spent listening, held still while the transcript is finished. */
function useListeningClock(status: DictationStatus): number {
  const [elapsed, setElapsed] = useState(0)
  const startedAt = useRef<number | null>(null)
  useEffect(() => {
    if (status !== 'listening') return
    startedAt.current ??= performance.now()
    const tick = () => setElapsed((performance.now() - (startedAt.current ?? 0)) / 1000)
    tick()
    const timer = window.setInterval(tick, 250)
    return () => window.clearInterval(timer)
  }, [status])
  return elapsed
}
