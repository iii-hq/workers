/**
 * The `voice-dictate` composer action: the mic beside the attach button.
 * Click toggles listening and holding past 400 ms is push-to-talk; the
 * console's Dictate command (Ctrl+Shift+D on a Mac, Alt+Shift+D elsewhere)
 * does the same from the keyboard: tap to start and tap to stop, or hold
 * the chord and talk, and the text lands when you let go. The right
 * Option key on its own is the one-key version of the same gesture (see
 * `lib/dictation-key`): tap it to start and stop, or hold it and talk.
 * While it listens the toolbar becomes the `RecordingBar`: a live
 * waveform, the time, cancel and done. The transcript, cleaned up, goes to
 * the composer through `host.chat.compose` when listening ends. Consoles without the toolbar slot never call this;
 * the chip and the live row carry the mic there.
 */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { doctor } from '../lib/client'
import type { DictationController } from '../lib/dictation'
import { useDictation, useDictationLevel } from '../lib/dictation'
import { createDictationKeyGesture } from '../lib/dictation-key'
import { deliverTranscript, dictationShortcutLabel, MicButton, toggleDictation, useMicPointer } from '../lib/mic'
import type { ComposerActionProps, ComposerActionRegistration } from '../lib/types'
import { RecordingBar } from './RecordingBar'

/** A chord held this long before release is push-to-talk; shorter is a tap that toggles. */
const KEY_HOLD_MS = 500
/** Key auto-repeat re-fires the command while the chord is held; calls this close together are one press. */
const KEY_REPEAT_MS = 700
/** The keys whose release ends a held chord. */
const CHORD_KEYS = new Set(['d', 'D', 'Shift', 'Control', 'Alt', 'Meta'])

export function createVoiceComposerAction(host: Host, controller: DictationController): ComposerActionRegistration {
  const finish = (text: string) => {
    void deliverTranscript(host, text)
  }
  let lastRunAt = 0
  /** When the running dictation was started from the keyboard, else null. */
  let keyStartedAt: number | null = null

  const onCommand = () => {
    const now = Date.now()
    const repeat = now - lastRunAt < KEY_REPEAT_MS
    lastRunAt = now
    if (repeat) return
    const status = controller.getState().status
    if (status === 'idle' || status === 'error') keyStartedAt = now
    else keyStartedAt = null
    void toggleDictation(controller, finish)
  }

  const isListening = () => {
    const status = controller.getState().status
    return status === 'listening' || status === 'starting'
  }

  /** Right Option alone: a tap toggles, a hold talks until release. */
  const optionKey = createDictationKeyGesture({
    toggle: () => void toggleDictation(controller, finish),
    holdStart: () => {
      if (!isListening()) void toggleDictation(controller, finish)
    },
    holdEnd: () => {
      if (isListening()) void toggleDictation(controller, finish)
    },
  })

  const onKeyDown = (event: KeyboardEvent) => optionKey.keydown(event)

  /** Releasing a chord held past the threshold stops and delivers, like lifting a finger off the mic. */
  const onKeyUp = (event: KeyboardEvent) => {
    optionKey.keyup(event)
    if (keyStartedAt === null || !CHORD_KEYS.has(event.key)) return
    const held = Date.now() - keyStartedAt >= KEY_HOLD_MS
    const status = controller.getState().status
    if (!held || (status !== 'listening' && status !== 'starting')) return
    keyStartedAt = null
    lastRunAt = Date.now()
    void toggleDictation(controller, finish)
  }

  function VoiceComposerAction({ isStreaming }: ComposerActionProps) {
    const [available, setAvailable] = useState(false)
    const pointer = useMicPointer(controller, finish)
    const { state } = useDictation(controller)
    const level = useDictationLevel(controller)

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

    useEffect(() => {
      window.addEventListener('keydown', onKeyDown, true)
      window.addEventListener('keyup', onKeyUp, true)
      window.addEventListener('blur', optionKey.blur)
      return () => {
        window.removeEventListener('keydown', onKeyDown, true)
        window.removeEventListener('keyup', onKeyUp, true)
        window.removeEventListener('blur', optionKey.blur)
      }
    }, [])

    if (!available) return null

    const className = ['voice-composer-action', isStreaming ? 'streaming' : ''].filter(Boolean).join(' ')
    const recording = state.status === 'starting' || state.status === 'listening' || state.status === 'stopping'
    return (
      <span className={className} data-status={state.status} data-recording={recording || undefined}>
        {recording ? (
          <RecordingBar
            controller={controller}
            status={state.status}
            holding={pointer.holding}
            onDone={() => void toggleDictation(controller, finish)}
            onCancel={() => void controller.cancel()}
          />
        ) : null}
        <MicButton pointer={pointer} className="voice-composer-btn" phase={state.status} level={level} shortcut={dictationShortcutLabel()} />
      </span>
    )
  }

  return {
    id: 'voice-dictate',
    render: VoiceComposerAction,
    command: {
      title: 'Dictate',
      detail: 'Tap to start and stop, or hold to talk; the right Option key on its own does the same. The text lands in the message box',
      keywords: ['voice', 'mic', 'microphone', 'speech', 'record', 'talk'],
      shortcut: { mac: ['Ctrl+Shift+D'], other: ['Alt+Shift+D'] },
      firesWhileTyping: true,
      run: onCommand,
    },
  }
}
