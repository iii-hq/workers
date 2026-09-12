import type { StatusChangedEvent } from '@/lib/sessions/types'

const ENABLED_KEY = 'iii-completion-bell-enabled'
const CLAIMS_KEY = 'iii-completion-bell-claims'
const PREFERENCE_EVENT = 'iii-completion-bell-preference'
const CLAIM_MAX_AGE_MS = 10 * 60 * 1_000
const LOCK_NAME = 'iii-console-completion-bell'

export type CompletionBellKind = 'completed' | 'failed'

export interface BellConversation {
  parentId?: string
  depth?: number
  status?: StatusChangedEvent['status']
  serverStatusUpdatedAt?: number
}

type AudioContextConstructor = new () => AudioContext
type NavigatorWithLocks = Navigator & {
  locks?: {
    request<T>(name: string, callback: () => T | PromiseLike<T>): Promise<T>
  }
}

let audioContext: AudioContext | null = null

export function loadCompletionBellEnabled(): boolean {
  if (typeof window === 'undefined') return true
  try {
    return localStorage.getItem(ENABLED_KEY) !== 'false'
  } catch {
    return true
  }
}

export function saveCompletionBellEnabled(enabled: boolean): void {
  try {
    localStorage.setItem(ENABLED_KEY, String(enabled))
  } catch {
    /* best-effort browser preference */
  }
  window.dispatchEvent(new Event(PREFERENCE_EVENT))
}

export function subscribeCompletionBellPreference(
  listener: () => void,
): () => void {
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

function audioContextConstructor(): AudioContextConstructor | undefined {
  if (typeof window === 'undefined') return undefined
  return (
    window.AudioContext ??
    (
      window as typeof window & {
        webkitAudioContext?: AudioContextConstructor
      }
    ).webkitAudioContext
  )
}

/** Unlock Web Audio from a user gesture so a later completion can ring. */
export async function unlockCompletionBell(): Promise<boolean> {
  const AudioContextImpl = audioContextConstructor()
  if (!AudioContextImpl) return false

  try {
    audioContext ??= new AudioContextImpl()
    if (audioContext.state === 'suspended') await audioContext.resume()
    return audioContext.state === 'running'
  } catch (error) {
    console.warn('[iii-bell] could not unlock completion audio', error)
    return false
  }
}

/** Arm audio on the first interaction; browsers otherwise block later sound. */
export function armCompletionBell(): () => void {
  if (typeof window === 'undefined') return () => undefined

  const unlock = () => {
    cleanup()
    void unlockCompletionBell()
  }
  const cleanup = () => {
    window.removeEventListener('pointerdown', unlock, true)
    window.removeEventListener('keydown', unlock, true)
  }

  window.addEventListener('pointerdown', unlock, {
    capture: true,
    once: true,
  })
  window.addEventListener('keydown', unlock, { capture: true, once: true })
  return cleanup
}

function scheduleTone(
  context: AudioContext,
  frequency: number,
  startsAt: number,
  duration: number,
  peak: number,
): void {
  const oscillator = context.createOscillator()
  const gain = context.createGain()
  oscillator.type = 'sine'
  oscillator.frequency.setValueAtTime(frequency, startsAt)
  gain.gain.setValueAtTime(0.0001, startsAt)
  gain.gain.exponentialRampToValueAtTime(peak, startsAt + 0.018)
  gain.gain.exponentialRampToValueAtTime(0.0001, startsAt + duration)
  oscillator.connect(gain)
  gain.connect(context.destination)
  oscillator.start(startsAt)
  oscillator.stop(startsAt + duration + 0.02)
}

/** Play a short synthesized bell; no audio asset or notification permission. */
export async function playCompletionBell(
  kind: CompletionBellKind = 'completed',
  force = false,
): Promise<boolean> {
  if (!force && !loadCompletionBellEnabled()) return false
  if (!(await unlockCompletionBell()) || !audioContext) return false

  const startsAt = audioContext.currentTime + 0.015
  if (kind === 'failed') {
    scheduleTone(audioContext, 392, startsAt, 0.2, 0.07)
    scheduleTone(audioContext, 293.66, startsAt + 0.16, 0.32, 0.06)
  } else {
    scheduleTone(audioContext, 783.99, startsAt, 0.18, 0.055)
    scheduleTone(audioContext, 1046.5, startsAt + 0.13, 0.36, 0.05)
  }
  return true
}

export function shouldRingCompletionBell(
  event: StatusChangedEvent,
  conversation: BellConversation | undefined,
): boolean {
  if (!conversation) return false
  if (conversation.parentId || (conversation.depth ?? 0) > 0) return false
  if (conversation.status !== 'working') return false
  if (
    conversation.serverStatusUpdatedAt !== undefined &&
    event.timestamp < conversation.serverStatusUpdatedAt
  ) {
    return false
  }
  if (event.status_reason === 'stopped') return false
  if (event.previous_status !== 'working') return false
  if (event.status !== 'done' && event.status !== 'error') return false
  return true
}

function claimWithoutLock(eventKey: string): boolean {
  if (typeof localStorage === 'undefined') return true
  try {
    const now = Date.now()
    const stored = localStorage.getItem(CLAIMS_KEY)
    const parsed = stored ? (JSON.parse(stored) as Record<string, number>) : {}
    const claims = Object.fromEntries(
      Object.entries(parsed).filter(
        ([, timestamp]) =>
          typeof timestamp === 'number' && timestamp >= now - CLAIM_MAX_AGE_MS,
      ),
    )
    if (claims[eventKey] !== undefined) return false
    claims[eventKey] = now
    localStorage.setItem(CLAIMS_KEY, JSON.stringify(claims))
    return true
  } catch {
    return true
  }
}

async function claimCompletionEvent(eventKey: string): Promise<boolean> {
  const locks = (navigator as NavigatorWithLocks).locks
  if (!locks) return claimWithoutLock(eventKey)
  return locks.request(LOCK_NAME, () => claimWithoutLock(eventKey))
}

/** Claim and ring once for one live top-level terminal status event. */
export async function ringForCompletionEvent(
  event: StatusChangedEvent,
  conversation: BellConversation | undefined,
): Promise<boolean> {
  if (!loadCompletionBellEnabled()) return false
  if (!shouldRingCompletionBell(event, conversation)) return false
  const eventKey = `${event.session_id}:${event.status}:${event.timestamp}`
  if (!(await claimCompletionEvent(eventKey))) return false
  return playCompletionBell(event.status === 'error' ? 'failed' : 'completed')
}
