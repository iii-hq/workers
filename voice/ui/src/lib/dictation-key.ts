/**
 * The one-key dictation gesture on the right Option (Alt) key: tap it to
 * start and tap again to stop, or hold it and talk and let go to finish —
 * the way an OS dictation key works.
 *
 * Right Option because it is the one lone key a browser hears on a Mac:
 * Chromium and WebKit drop the bare fn (globe) key before the page sees
 * it. Right Option also types characters (⌥E, ⌥8…), so only a clean press
 * counts: any other key while it is down cancels the gesture, and a
 * dictation only starts on release or once it has been held alone.
 */

/** Held alone this long, the press is push-to-talk rather than a tap. */
export const DICTATION_KEY_HOLD_MS = 350

export interface DictationKeyActions {
  /** A clean tap: start when idle, stop and deliver when listening. */
  toggle(): void
  /** The key has been held alone past the threshold: start listening. */
  holdStart(): void
  /** The held key was released: stop and deliver. */
  holdEnd(): void
}

interface KeyLike {
  key: string
  code: string
  repeat?: boolean
  metaKey: boolean
  ctrlKey: boolean
  shiftKey: boolean
}

/**
 * The right Option key on its own. AltGr layouts report `AltGraph` (Linux)
 * or Control+Alt (Windows), so they never match.
 */
export function isDictationKey(event: KeyLike): boolean {
  return event.code === 'AltRight' && event.key === 'Alt' && !event.metaKey && !event.ctrlKey && !event.shiftKey
}

export function createDictationKeyGesture(
  actions: DictationKeyActions,
  timers: { set: (fn: () => void, ms: number) => number; clear: (id: number) => void } = {
    set: (fn, ms) => window.setTimeout(fn, ms),
    clear: (id) => window.clearTimeout(id),
  },
) {
  /** The key is down and nothing else has been pressed with it. */
  let clean = false
  let holding = false
  let timer: number | null = null

  const clearTimer = () => {
    if (timer !== null) timers.clear(timer)
    timer = null
  }

  const reset = () => {
    clearTimer()
    if (holding) actions.holdEnd()
    clean = false
    holding = false
  }

  return {
    keydown(event: KeyLike) {
      if (isDictationKey(event)) {
        if (event.repeat || clean || holding) return
        clean = true
        timer = timers.set(() => {
          timer = null
          if (!clean) return
          holding = true
          actions.holdStart()
        }, DICTATION_KEY_HOLD_MS)
        return
      }
      // Another key with Option down is a shortcut or a ⌥ character, not dictation.
      if (clean && !holding) {
        clean = false
        clearTimer()
      }
    },
    keyup(event: KeyLike) {
      if (event.code !== 'AltRight') return
      clearTimer()
      if (holding) actions.holdEnd()
      else if (clean) actions.toggle()
      clean = false
      holding = false
    },
    /** The window lost focus mid-press: the key-up will never come. */
    blur: reset,
  }
}
