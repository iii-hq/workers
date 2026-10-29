/**
 * The `voice-dictate` composer action: the mic beside the attach button.
 * Click toggles listening and holding past 400 ms is push-to-talk; the
 * console's Dictate command (Ctrl+Shift+D on a Mac, Alt+Shift+D elsewhere)
 * does the same from the keyboard: tap to start and tap to stop, or hold
 * the chord and talk, and the text lands when you let go. Holding fn or the
 * right Option key works anywhere, not just here (see `../keys`). While it
 * listens the toolbar becomes the `RecordingBar`: a live waveform, the time,
 * cancel and done. The transcript, cleaned up, goes where the person was
 * typing, else to the composer through `host.chat.compose`. Consoles
 * without the toolbar slot never call this; the chip and the live row carry
 * the mic there.
 */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { doctor } from '../lib/client'
import type { DictationController } from '../lib/dictation'
import { useDictation, useDictationLevel } from '../lib/dictation'
import { dictationShortcutLabel, MicButton, toggleDictation, useMicPointer } from '../lib/mic'
import { type DictationKeys, useFnAvailable } from '../keys'
import type { ComposerActionProps, ComposerActionRegistration } from '../lib/types'
import { RecordingBar } from './RecordingBar'

/** A chord held this long before release is push-to-talk; shorter is a tap that toggles. */
const KEY_HOLD_MS = 500
/** Key auto-repeat re-fires the command while the chord is held; calls this close together are one press. */
const KEY_REPEAT_MS = 700
/** The keys whose release ends a held chord. */
const CHORD_KEYS = new Set(['d', 'D', 'Shift', 'Control', 'Alt', 'Meta'])

export function createVoiceComposerAction(
  host: Host,
  controller: DictationController,
  keys: DictationKeys,
): ComposerActionRegistration {
  const finish = keys.finish
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
    keys.captureTarget()
    void toggleDictation(controller, finish)
  }

  /** Releasing a chord held past the threshold stops and delivers, like lifting a finger off the mic. */
  const onKeyUp = (event: KeyboardEvent) => {
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

    const fnReady = useFnAvailable(keys)

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
      window.addEventListener('keyup', onKeyUp, true)
      return () => window.removeEventListener('keyup', onKeyUp, true)
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
        <MicButton pointer={pointer} className="voice-composer-btn" phase={state.status} level={level} shortcut={dictationShortcutLabel(fnReady)} />
      </span>
    )
  }

  return {
    id: 'voice-dictate',
    render: VoiceComposerAction,
    command: {
      title: 'Dictate',
      detail:
        'Hold fn (or the right Option key) and talk; let go and the text is typed where your cursor is, or into the message box. A tap starts and stops',
      keywords: ['voice', 'mic', 'microphone', 'speech', 'record', 'talk'],
      shortcut: { mac: ['Ctrl+Shift+D'], other: ['Alt+Shift+D'] },
      firesWhileTyping: true,
      run: onCommand,
    },
  }
}
