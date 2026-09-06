/**
 * The `voice-dictate` composer action: the mic beside the attach button.
 * Click toggles listening, holding past 400 ms is push-to-talk; the
 * transcript goes to the composer through `host.chat.compose` when
 * listening ends. Live partial text shows in the chat-header chip. Consoles
 * without the toolbar slot never call this; the chip carries the mic there.
 */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { doctor } from '../lib/client'
import type { DictationController } from '../lib/dictation'
import { deliverTranscript, MicButton, useMicPointer } from '../lib/mic'
import type { ComposerActionProps, ComposerActionRegistration } from '../lib/types'

export function createVoiceComposerAction(host: Host, controller: DictationController): ComposerActionRegistration {
  function VoiceComposerAction({ isStreaming }: ComposerActionProps) {
    const [available, setAvailable] = useState(false)
    const pointer = useMicPointer(controller, (text) => {
      deliverTranscript(host, text)
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
      <span className={isStreaming ? 'voice-composer-action streaming' : 'voice-composer-action'}>
        <MicButton pointer={pointer} className="voice-composer-btn" />
      </span>
    )
  }

  return { id: 'voice-dictate', render: VoiceComposerAction }
}
