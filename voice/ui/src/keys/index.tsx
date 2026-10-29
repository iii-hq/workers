/**
 * Dictation from the keyboard, anywhere in the console, the way Wispr Flow
 * works: hold fn (or the right Option key), talk, let go, and the words are
 * typed where your cursor is — the IDE's editor or terminal, a search box —
 * or into the chat composer when no field has focus. A quick tap starts
 * hands-free dictation and a second tap finishes it.
 *
 * Installed once by `setup`, so it works whatever is on screen, chat or
 * not. fn is heard by the worker (`voice::hotkey`; browsers never see a bare
 * fn), right Option by the page. While a key dictation runs and the
 * composer's own recording bar is not on screen, a small pill at the bottom
 * of the window shows it listening.
 */

import type { Host } from '@iii-dev/console-ui'
import { useSyncExternalStore } from 'react'
import { createRoot } from 'react-dom/client'
import { doctor } from '../lib/client'
import { type DictationController, useDictation } from '../lib/dictation'
import { createDictationKeyGesture, createHoldGesture } from '../lib/dictation-key'
import { deepActiveElement, editableTarget } from '../lib/focus-target'
import { LiveWaveform } from '../lib/live-waveform'
import { deliverTranscript, toggleDictation } from '../lib/mic'

/** fn held this long is push-to-talk: short enough to feel instant, long
    enough that fn+arrow or fn+delete never opens the mic. */
const FN_HOLD_MS = 180
const HOTKEY_FN = 'iii::voice-ui::hotkey'

interface HotkeyEvent {
  key: string
  action: 'down' | 'up' | 'chord'
}

export interface DictationKeys {
  /** Remember the focused field, as a key or command starts dictation. */
  captureTarget(): void
  /** Deliver finished text to the remembered field, else the composer. */
  finish(text: string): void
  /** Whether the worker hears fn (for tooltips). */
  fnAvailable(): boolean
  subscribeFn(listener: () => void): () => void
}

export function installDictationKeys(host: Host, controller: DictationController): DictationKeys & { dispose(): void } {
  /** The field that had focus when the running dictation began. */
  let target: HTMLElement | null = null
  /** The running dictation came from a key: the pill may show it. */
  let fromKey = false
  let fnReady = false
  const fnListeners = new Set<() => void>()

  const isIdle = () => {
    const status = controller.getState().status
    return status === 'idle' || status === 'error'
  }
  const isListening = () => {
    const status = controller.getState().status
    return status === 'listening' || status === 'starting'
  }

  const captureTarget = () => {
    if (isIdle()) target = editableTarget(deepActiveElement())
  }
  const finish = (text: string) => {
    const field = target
    target = null
    void deliverTranscript(host, text, field)
  }
  const keyToggle = () => {
    if (isIdle()) {
      captureTarget()
      fromKey = true
    }
    void toggleDictation(controller, finish)
  }

  const actions = {
    toggle: keyToggle,
    holdStart: () => {
      if (!isListening()) keyToggle()
    },
    holdEnd: () => {
      if (isListening()) keyToggle()
    },
  }
  const optionKey = createDictationKeyGesture(actions)
  const fnKey = createHoldGesture(actions, FN_HOLD_MS)

  // The worker hears fn in every app; only the window in front dictates.
  const onHotkey = (event: HotkeyEvent) => {
    if (event.key !== 'fn') return
    if (event.action === 'down') {
      if (document.visibilityState === 'visible' && document.hasFocus()) fnKey.press()
    } else if (event.action === 'up') fnKey.release()
    else fnKey.interrupt()
  }
  const localFn = `${HOTKEY_FN}::${Math.random().toString(36).slice(2)}`
  const offHandler = host.iii.on<HotkeyEvent>(localFn, onHotkey)
  let offTrigger: (() => void) | null = null
  try {
    offTrigger = host.iii.registerTrigger({
      type: 'voice::hotkey',
      function_id: `${localFn}::${host.iii.browserId}`,
      config: {},
    })
  } catch {
    // A worker without the fn key: the right Option key still works.
  }
  void doctor(host.iii)
    .then((report) => {
      fnReady = report.fn_key?.available === true
      for (const listener of fnListeners) listener()
    })
    .catch(() => {})

  const onKeyDown = (event: KeyboardEvent) => optionKey.keydown(event)
  const onKeyUp = (event: KeyboardEvent) => optionKey.keyup(event)
  const onBlur = () => {
    optionKey.blur()
    fnKey.reset()
  }
  window.addEventListener('keydown', onKeyDown, true)
  window.addEventListener('keyup', onKeyUp, true)
  window.addEventListener('blur', onBlur)

  // The pill, in a root of its own under the worker's CSS scope.
  const pillHost = document.createElement('div')
  pillHost.dataset.iiiUi = 'voice'
  pillHost.className = 'voice-pill-host'
  document.body.appendChild(pillHost)
  const root = createRoot(pillHost)
  const offStatus = controller.subscribe(() => {
    if (isIdle()) fromKey = false
  })
  root.render(<DictationPill controller={controller} fromKey={() => fromKey} />)

  return {
    captureTarget,
    finish,
    fnAvailable: () => fnReady,
    subscribeFn: (listener) => {
      fnListeners.add(listener)
      return () => fnListeners.delete(listener)
    },
    dispose: () => {
      window.removeEventListener('keydown', onKeyDown, true)
      window.removeEventListener('keyup', onKeyUp, true)
      window.removeEventListener('blur', onBlur)
      offTrigger?.()
      offHandler()
      offStatus()
      root.unmount()
      pillHost.remove()
    },
  }
}

/** The fn availability, for a tooltip that names the right key. */
export function useFnAvailable(keys: DictationKeys): boolean {
  return useSyncExternalStore(keys.subscribeFn, keys.fnAvailable, () => false)
}

/** The composer's own recording bar is on screen: it shows the dictation. */
function composerBarVisible(): boolean {
  const bar = document.querySelector('.voice-recording-bar')
  if (!(bar instanceof HTMLElement)) return false
  const box = bar.getBoundingClientRect()
  return box.width > 0 && box.height > 0 && box.bottom > 0 && box.top < window.innerHeight
}

function DictationPill({ controller, fromKey }: { controller: DictationController; fromKey: () => boolean }) {
  const { state } = useDictation(controller)
  const active = state.status === 'starting' || state.status === 'listening' || state.status === 'stopping'
  if (!active || !fromKey() || composerBarVisible()) return null
  const stopping = state.status === 'stopping'
  return (
    <div className="voice-pill" data-status={state.status} role="status" aria-live="polite">
      <LiveWaveform
        className="voice-pill-wave"
        active={state.status === 'listening'}
        processing={stopping}
        getAnalyser={controller.getAnalyser}
        fadeWidth={16}
      />
      <span className="voice-pill-label">{stopping ? 'Writing…' : 'Listening'}</span>
    </div>
  )
}
