/**
 * The `voice-mic` session chip: the mic button in the chat header, used only
 * on consoles that predate the composer toolbar slot. Click toggles
 * listening, holding past 400 ms is push-to-talk; the transcript lands in
 * the composer via `host.chat.compose` (a trailing space, never
 * auto-submitted), or on the clipboard when that helper is absent. The chip
 * renders nothing until `voice::doctor` confirms the worker is present.
 */

import type { Host, SessionChipProps, SessionChipRegistration } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { doctor } from '../lib/client'
import type { DictationController } from '../lib/dictation'
import { truncate } from '../lib/format'
import { deliverTranscript, MicButton, useMicPointer } from '../lib/mic'

const MESSAGE_DISPLAY_MS = 4000
const PARTIAL_MAX_CHARS = 48

export function createVoiceSessionChip(host: Host, controller: DictationController): SessionChipRegistration {
  function VoiceMicChip(_props: SessionChipProps) {
    const [available, setAvailable] = useState(false)
    const [flash, setFlash] = useState<{ kind: 'copied' | 'failed'; text: string } | null>(null)
    const pointer = useMicPointer(controller, async (text) => {
      const delivered = await deliverTranscript(host, text)
      if (delivered === 'composer') return
      setFlash({ kind: delivered === 'clipboard' ? 'copied' : 'failed', text })
      window.setTimeout(() => setFlash(null), MESSAGE_DISPLAY_MS)
    })

    useEffect(() => {
      let cancelled = false
      doctor(host.iii)
        .then(() => {
          if (!cancelled) setAvailable(true)
        })
        .catch(() => {
          if (!cancelled) setAvailable(false)
        })
      return () => {
        cancelled = true
      }
    }, [])

    if (!available) return null

    return (
      <span className="voice-chip-root">
        <MicButton pointer={pointer} className="voice-chip-btn" />
        {!pointer.listening && flash?.kind === 'copied' ? (
          <span className="voice-chip-copied" title={flash.text}>
            {truncate(flash.text, PARTIAL_MAX_CHARS)} · copied
          </span>
        ) : null}
        {!pointer.listening && flash?.kind === 'failed' ? (
          <span className="voice-chip-error" title={flash.text}>
            clipboard blocked: {truncate(flash.text, PARTIAL_MAX_CHARS)}
          </span>
        ) : null}
        {!pointer.listening && pointer.errorFlash ? (
          <span className="voice-chip-error">{pointer.errorFlash}</span>
        ) : null}
      </span>
    )
  }

  return { id: 'voice-mic', render: VoiceMicChip }
}
