/**
 * The mic control shared by the composer action and the chat-header chip:
 * `useMicPointer` turns pointer events into click-to-toggle and
 * hold-to-talk (pointer held past 400 ms starts, release stops and hands
 * the transcript to `onFinish`), and `MicButton` is the `IconButton` it
 * drives, with the accessible name, the listening state and the inline
 * error tooltip.
 */

import { IconButton } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { DictationController } from './dictation'
import { useDictation } from './dictation'
import { MicIcon } from './icons'

const HOLD_THRESHOLD_MS = 400
const MESSAGE_DISPLAY_MS = 4000

export interface MicPointer {
  listening: boolean
  errorFlash: string | null
  onPointerDown: () => void
  onPointerUp: () => void
  onPointerCancel: () => void
}

export function useMicPointer(controller: DictationController, onFinish: (text: string) => void): MicPointer {
  const { state, start, stop, cancel } = useDictation(controller)
  const [errorFlash, setErrorFlash] = useState<string | null>(null)
  const holdTimerRef = useRef<number | null>(null)
  const heldRef = useRef(false)
  const pendingRef = useRef<'finish' | 'cancel' | null>(null)
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
  const canStart = state.status === 'idle' || state.status === 'error'

  const finish = useCallback(async () => {
    const text = (await stop()).trim()
    if (text) onFinish(text)
  }, [stop, onFinish])

  useEffect(() => {
    if (state.status === 'listening' && pendingRef.current) {
      const pending = pendingRef.current
      pendingRef.current = null
      if (pending === 'finish') finish()
      else cancel()
    } else if (state.status === 'idle' || state.status === 'error') {
      pendingRef.current = null
    }
  }, [state.status, finish, cancel])

  function clearHold() {
    if (holdTimerRef.current !== null) {
      window.clearTimeout(holdTimerRef.current)
      holdTimerRef.current = null
    }
  }

  return {
    listening,
    errorFlash,
    onPointerDown() {
      heldRef.current = false
      holdTimerRef.current = window.setTimeout(() => {
        heldRef.current = true
        if (canStart) start()
      }, HOLD_THRESHOLD_MS)
    },
    onPointerUp() {
      clearHold()
      if (heldRef.current) {
        heldRef.current = false
        if (state.status === 'listening') finish()
        else if (state.status === 'starting') pendingRef.current = 'finish'
        return
      }
      if (canStart) start()
      else if (state.status === 'listening') finish()
    },
    onPointerCancel() {
      clearHold()
      if (heldRef.current) {
        heldRef.current = false
        if (state.status === 'listening') cancel()
        else if (state.status === 'starting') pendingRef.current = 'cancel'
      }
    },
  }
}

export function MicButton({ pointer, className }: { pointer: MicPointer; className: string }) {
  const { listening, errorFlash } = pointer
  return (
    <IconButton
      label="Dictate"
      tooltip={errorFlash ?? (listening ? 'listening; click or release to stop' : 'Dictate')}
      title={errorFlash ?? undefined}
      aria-pressed={listening}
      className={listening ? `${className} listening` : className}
      onPointerDown={pointer.onPointerDown}
      onPointerUp={pointer.onPointerUp}
      onPointerLeave={pointer.onPointerCancel}
      onPointerCancel={pointer.onPointerCancel}
      onContextMenu={(event: { preventDefault(): void }) => event.preventDefault()}
    >
      <MicIcon />
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
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    return 'failed'
  }
  return 'clipboard'
}
