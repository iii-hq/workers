/**
 * The mic control shared by the composer action and the chat-header chip:
 * `useMicPointer` turns pointer events into click-to-toggle and
 * hold-to-talk (pointer held past 400 ms starts, release stops and hands
 * the transcript to `onFinish`), and `MicButton` is the `IconButton` it
 * drives, with the accessible name, the listening state and the inline
 * error tooltip.
 */

import { IconButton } from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import { Mic } from 'lucide-react'
import type React from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { DictationController } from './dictation'
import { useDictation } from './dictation'

const HOLD_THRESHOLD_MS = 400
const MESSAGE_DISPLAY_MS = 4000

/**
 * Start a dictation, or stop the running one and hand its text to
 * `onFinish`. What the mic's click does, for a key or a palette row that
 * has no pointer: the same controller, so the mic's own state follows.
 */
export async function toggleDictation(
  controller: DictationController,
  onFinish: (text: string) => void,
): Promise<void> {
  const status = controller.getState().status
  if (status === 'idle' || status === 'error') {
    await controller.start()
    return
  }
  if (status === 'listening' || status === 'starting') {
    const text = (await controller.stop()).trim()
    if (text) onFinish(text)
  }
}

/** The one key that toggles dictation, for tooltips (see `lib/dictation-key`). */
export function dictationShortcutLabel(): string {
  const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/.test(navigator.userAgent)
  return mac ? 'Right ⌥' : 'Right Alt'
}

export interface MicPointer {
  listening: boolean
  /** The pointer is held down on the mic: push-to-talk, ending on release. */
  holding: boolean
  errorFlash: string | null
  onPointerDown: () => void
  onPointerUp: () => void
  onPointerCancel: () => void
}

export function useMicPointer(controller: DictationController, onFinish: (text: string) => void): MicPointer {
  const { state, start, stop, cancel } = useDictation(controller)
  const [errorFlash, setErrorFlash] = useState<string | null>(null)
  const [holding, setHolding] = useState(false)
  const holdTimerRef = useRef<number | null>(null)
  const heldRef = useRef(false)
  const messageTimerRef = useRef<number | null>(null)

  useEffect(() => {
    if (state.status !== 'error' || !state.error) return
    setErrorFlash(state.error)
    if (messageTimerRef.current !== null) window.clearTimeout(messageTimerRef.current)
    messageTimerRef.current = window.setTimeout(() => setErrorFlash(null), MESSAGE_DISPLAY_MS)
  }, [state.status, state.error])

  useEffect(
    () => () => {
      if (holdTimerRef.current !== null) window.clearTimeout(holdTimerRef.current)
      if (messageTimerRef.current !== null) window.clearTimeout(messageTimerRef.current)
    },
    [],
  )

  const listening = state.status === 'listening' || state.status === 'starting'

  const finish = useCallback(async () => {
    const text = (await stop()).trim()
    if (text) onFinish(text)
  }, [stop, onFinish])

  function clearHold() {
    if (holdTimerRef.current !== null) {
      window.clearTimeout(holdTimerRef.current)
      holdTimerRef.current = null
    }
    setHolding(false)
  }

  return {
    listening,
    holding,
    errorFlash,
    onPointerDown() {
      heldRef.current = false
      holdTimerRef.current = window.setTimeout(() => {
        heldRef.current = true
        setHolding(true)
        const status = controller.getState().status
        if (status === 'idle' || status === 'error') void start()
      }, HOLD_THRESHOLD_MS)
    },
    onPointerUp() {
      clearHold()
      const status = controller.getState().status
      if (heldRef.current) {
        heldRef.current = false
        if (status === 'listening' || status === 'starting') void finish()
        return
      }
      if (status === 'idle' || status === 'error') void start()
      else if (status === 'listening' || status === 'starting') void finish()
    },
    onPointerCancel() {
      clearHold()
      if (heldRef.current) {
        heldRef.current = false
        const status = controller.getState().status
        if (status === 'listening' || status === 'starting') void cancel()
      }
    },
  }
}

export function MicButton({
  pointer,
  className,
  phase,
  level = 0,
  shortcut,
}: {
  pointer: MicPointer
  className: string
  /** The controller's status, for the recording, starting and finishing looks; `listening` alone otherwise. */
  phase?: 'idle' | 'starting' | 'listening' | 'stopping' | 'error'
  /** Microphone level 0 to 1; the glow follows it. */
  level?: number
  /** Key label shown in the idle tooltip. */
  shortcut?: string
}) {
  const { listening, errorFlash } = pointer
  const state = phase ?? (listening ? 'listening' : 'idle')
  const classes = [className, listening ? 'listening' : '', `is-${state}`].filter(Boolean).join(' ')
  const idleTip = shortcut ? `Dictate · ${shortcut}` : 'Dictate'
  const listeningTip = shortcut ? `Listening · ${shortcut} or click to stop` : 'Listening · click to stop'
  const tip = errorFlash ?? (state === 'stopping' ? 'Finishing…' : listening ? listeningTip : idleTip)
  return (
    <IconButton
      label="Dictate"
      tooltip={tip}
      title={errorFlash ?? undefined}
      aria-pressed={listening}
      style={{ '--voice-level': level } as React.CSSProperties}
      className={classes}
      onPointerDown={pointer.onPointerDown}
      onPointerUp={pointer.onPointerUp}
      onPointerLeave={pointer.onPointerCancel}
      onPointerCancel={pointer.onPointerCancel}
      onContextMenu={(event: { preventDefault(): void }) => event.preventDefault()}
    >
      <Mic />
    </IconButton>
  )
}

/** Hand dictated text to the composer, or the clipboard when no composer slot exists. */
export async function deliverTranscript(
  host: { chat?: { compose?: (draft: { text?: string }) => void } },
  text: string,
): Promise<'composer' | 'clipboard' | 'failed'> {
  if (host.chat?.compose) {
    host.chat.compose({ text: `${text} ` })
    return 'composer'
  }
  return (await copyText(text)) ? 'clipboard' : 'failed'
}
