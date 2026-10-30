/**
 * Interface sounds: short synthesized cues for a few deliberate choices
 * (opening the model picker, switching models). Synthesized with Web Audio
 * like the completion bell, so there is no asset to load, and quiet enough
 * to sit under typing. The "Interface sounds" setting turns them off; a
 * blocked or missing audio context simply plays nothing.
 */

const ENABLED_KEY = 'iii-ui-sounds-enabled'
const PREFERENCE_EVENT = 'iii-ui-sounds-preference'

/** Two cues of the same kind closer than this play once. */
const MIN_GAP_MS = 45

export type UiSound = 'tick' | 'switch'

type AudioContextConstructor = new () => AudioContext

let context: AudioContext | null = null
const lastPlayed = new Map<UiSound, number>()

export function loadUiSoundsEnabled(): boolean {
  if (typeof window === 'undefined') return true
  try {
    return localStorage.getItem(ENABLED_KEY) !== 'false'
  } catch {
    return true
  }
}

export function saveUiSoundsEnabled(enabled: boolean): void {
  try {
    localStorage.setItem(ENABLED_KEY, String(enabled))
  } catch {
    /* best-effort browser preference */
  }
  window.dispatchEvent(new Event(PREFERENCE_EVENT))
}

export function subscribeUiSoundsPreference(listener: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === ENABLED_KEY) listener()
  }
  window.addEventListener(PREFERENCE_EVENT, listener)
  window.addEventListener('storage', onStorage)
  return () => {
    window.removeEventListener(PREFERENCE_EVENT, listener)
    window.removeEventListener('storage', onStorage)
  }
}

function audio(): AudioContext | null {
  if (typeof window === 'undefined') return null
  const Impl: AudioContextConstructor | undefined =
    window.AudioContext ??
    (window as typeof window & { webkitAudioContext?: AudioContextConstructor })
      .webkitAudioContext
  if (!Impl) return null
  try {
    context ??= new Impl()
    // Cues only ever play from a click or key press, which unlocks audio.
    if (context.state === 'suspended') void context.resume()
    return context
  } catch {
    return null
  }
}

/**
 * One voice: an oscillator gliding from `from` to `to` Hz under a fast
 * attack and an exponential tail, which reads as a soft "tock" rather
 * than a beep.
 */
function voice(
  ctx: AudioContext,
  type: OscillatorType,
  from: number,
  to: number,
  startsAt: number,
  duration: number,
  peak: number,
): void {
  const osc = ctx.createOscillator()
  const gain = ctx.createGain()
  osc.type = type
  osc.frequency.setValueAtTime(from, startsAt)
  osc.frequency.exponentialRampToValueAtTime(to, startsAt + duration * 0.6)
  gain.gain.setValueAtTime(0.0001, startsAt)
  gain.gain.exponentialRampToValueAtTime(peak, startsAt + 0.004)
  gain.gain.exponentialRampToValueAtTime(0.0001, startsAt + duration)
  osc.connect(gain)
  gain.connect(ctx.destination)
  osc.start(startsAt)
  osc.stop(startsAt + duration + 0.02)
}

/** Play an interface cue; a no-op when sounds are off or audio is blocked. */
export function playUiSound(sound: UiSound): void {
  if (!loadUiSoundsEnabled()) return
  const now = performance.now()
  if (now - (lastPlayed.get(sound) ?? -Infinity) < MIN_GAP_MS) return
  lastPlayed.set(sound, now)
  const ctx = audio()
  if (!ctx) return
  try {
    const t = ctx.currentTime + 0.005
    if (sound === 'tick') {
      // A dry tap: a short, high triangle falling a few semitones.
      voice(ctx, 'triangle', 1900, 1300, t, 0.045, 0.035)
    } else {
      // A rising two-step pop: the choice landed.
      voice(ctx, 'sine', 520, 700, t, 0.09, 0.05)
      voice(ctx, 'sine', 780, 1040, t + 0.06, 0.14, 0.04)
    }
  } catch {
    /* sound is decoration; never let it break the click */
  }
}
