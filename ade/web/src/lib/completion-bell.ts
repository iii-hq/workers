import type { StatusChangedEvent } from '@/lib/sessions/types'

const ENABLED_KEY = 'iii-completion-bell-enabled'
const CLAIMS_KEY = 'iii-completion-bell-claims'
const PREFERENCE_EVENT = 'iii-completion-bell-preference'
const CLAIM_MAX_AGE_MS = 10 * 60 * 1_000
const LOCK_NAME = 'iii-console-completion-bell'
const CLAIM_DATABASE_NAME = 'iii-completion-bell'
const CLAIM_STORE_NAME = 'claims'
const CLAIM_DATABASE_VERSION = 1
const RESERVATION_LEASE_MS = 30_000
const CLAIM_CHANNEL_NAME = 'iii-completion-bell-claims'

export type CompletionBellKind = 'completed' | 'failed'

export interface BellConversation {
  parentId?: string
  depth?: number
  /** True only after authoritative session metadata established hierarchy. */
  hierarchyResolved?: boolean
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

/** Reset the module singleton between isolated browser-behavior tests. */
export function __resetCompletionBellForTests(): void {
  audioContext = null
}

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

  try {
    const startsAt = audioContext.currentTime + 0.015
    if (kind === 'failed') {
      scheduleTone(audioContext, 392, startsAt, 0.2, 0.07)
      scheduleTone(audioContext, 293.66, startsAt + 0.16, 0.32, 0.06)
    } else {
      scheduleTone(audioContext, 783.99, startsAt, 0.18, 0.055)
      scheduleTone(audioContext, 1046.5, startsAt + 0.13, 0.36, 0.05)
    }
    return true
  } catch (error) {
    console.warn('[iii-bell] could not play completion audio', error)
    return false
  }
}

export function shouldRingCompletionBell(
  event: StatusChangedEvent,
  conversation: BellConversation | undefined,
): boolean {
  if (!conversation?.hierarchyResolved) return false
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
interface ClaimRecord {
  eventKey: string
  owner: string
  state: 'pending' | 'claimed'
  expiresAt: number
}

type ReservationResult =
  | { state: 'reserved' }
  | { state: 'claimed' }
  | { state: 'pending'; expiresAt: number }

function activeLocalClaims(now: number): Record<string, number> {
  if (typeof localStorage === 'undefined') return {}
  try {
    const stored = localStorage.getItem(CLAIMS_KEY)
    const parsed = stored ? (JSON.parse(stored) as Record<string, number>) : {}
    return Object.fromEntries(
      Object.entries(parsed).filter(
        ([, timestamp]) =>
          typeof timestamp === 'number' && timestamp >= now - CLAIM_MAX_AGE_MS,
      ),
    )
  } catch {
    return {}
  }
}

function hasActiveLocalClaim(eventKey: string): boolean {
  return activeLocalClaims(Date.now())[eventKey] !== undefined
}

function saveLocalClaim(eventKey: string): void {
  if (typeof localStorage === 'undefined') return
  try {
    const now = Date.now()
    localStorage.setItem(
      CLAIMS_KEY,
      JSON.stringify({ ...activeLocalClaims(now), [eventKey]: now }),
    )
  } catch {
    /* storage is best effort; successful playback still counts in this tab */
  }
}

function openClaimsDatabase(): Promise<IDBDatabase> {
  if (typeof indexedDB === 'undefined') {
    return Promise.reject(new Error('IndexedDB is unavailable'))
  }
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(CLAIM_DATABASE_NAME, CLAIM_DATABASE_VERSION)
    request.onupgradeneeded = () => {
      if (!request.result.objectStoreNames.contains(CLAIM_STORE_NAME)) {
        request.result.createObjectStore(CLAIM_STORE_NAME, {
          keyPath: 'eventKey',
        })
      }
    }
    request.onsuccess = () => resolve(request.result)
    request.onerror = () =>
      reject(request.error ?? new Error('Could not open completion claims'))
    request.onblocked = () =>
      reject(new Error('Completion claim database upgrade was blocked'))
  })
}

async function reserveIndexedDbClaim(
  eventKey: string,
  owner: string,
): Promise<ReservationResult> {
  const database = await openClaimsDatabase()
  return new Promise((resolve, reject) => {
    const transaction = database.transaction(CLAIM_STORE_NAME, 'readwrite')
    const store = transaction.objectStore(CLAIM_STORE_NAME)
    const request = store.get(eventKey)
    let result: ReservationResult = { state: 'claimed' }

    request.onsuccess = () => {
      const now = Date.now()
      const current = request.result as ClaimRecord | undefined
      if (current && current.expiresAt > now) {
        result =
          current.state === 'claimed'
            ? { state: 'claimed' }
            : { state: 'pending', expiresAt: current.expiresAt }
        return
      }
      store.put({
        eventKey,
        owner,
        state: 'pending',
        expiresAt: now + RESERVATION_LEASE_MS,
      } satisfies ClaimRecord)
      result = { state: 'reserved' }
    }
    transaction.oncomplete = () => {
      database.close()
      resolve(result)
    }
    transaction.onabort = () => {
      database.close()
      reject(transaction.error ?? new Error('Could not reserve completion'))
    }
  })
}

async function settleIndexedDbClaim(
  eventKey: string,
  owner: string,
  played: boolean,
): Promise<void> {
  const database = await openClaimsDatabase()
  return new Promise((resolve, reject) => {
    const transaction = database.transaction(CLAIM_STORE_NAME, 'readwrite')
    const store = transaction.objectStore(CLAIM_STORE_NAME)
    const request = store.get(eventKey)

    request.onsuccess = () => {
      const current = request.result as ClaimRecord | undefined
      if (current?.state !== 'pending' || current.owner !== owner) return
      if (played) {
        store.put({
          eventKey,
          owner,
          state: 'claimed',
          expiresAt: Date.now() + CLAIM_MAX_AGE_MS,
        } satisfies ClaimRecord)
      } else {
        store.delete(eventKey)
      }
    }
    transaction.oncomplete = () => {
      database.close()
      resolve()
    }
    transaction.onabort = () => {
      database.close()
      reject(transaction.error ?? new Error('Could not settle completion'))
    }
  })
}

function claimOwner(): string {
  if (
    typeof crypto !== 'undefined' &&
    typeof crypto.randomUUID === 'function'
  ) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random()}`
}

interface ClaimWaiter {
  close: () => void
  wait: (expiresAt: number) => Promise<void>
}

function createClaimWaiter(eventKey: string): ClaimWaiter {
  let channel: BroadcastChannel | undefined
  let wake: (() => void) | undefined
  try {
    if (typeof BroadcastChannel !== 'undefined') {
      channel = new BroadcastChannel(CLAIM_CHANNEL_NAME)
      channel.onmessage = (message) => {
        if (message.data === eventKey) wake?.()
      }
    }
  } catch {
    channel = undefined
  }

  return {
    close: () => channel?.close(),
    wait: (expiresAt) =>
      new Promise((resolve) => {
        let finished = false
        const finish = () => {
          if (finished) return
          finished = true
          clearTimeout(timer)
          wake = undefined
          resolve()
        }
        const timer = setTimeout(
          finish,
          Math.max(0, expiresAt - Date.now() + 25),
        )
        wake = finish
      }),
  }
}

function notifyClaimChanged(eventKey: string): void {
  if (typeof BroadcastChannel === 'undefined') return
  try {
    const channel = new BroadcastChannel(CLAIM_CHANNEL_NAME)
    channel.postMessage(eventKey)
    channel.close()
  } catch {
    /* lease expiry remains the wake-up fallback */
  }
}

async function ringWithIndexedDbClaim(
  eventKey: string,
  kind: CompletionBellKind,
): Promise<boolean> {
  if (hasActiveLocalClaim(eventKey)) return false
  const owner = claimOwner()
  const waiter = createClaimWaiter(eventKey)
  let reservation: ReservationResult
  try {
    reservation = await reserveIndexedDbClaim(eventKey, owner)
  } catch {
    waiter.close()
    // Storage failure must not turn a playable completion into silence.
    return playCompletionBell(kind)
  }

  if (reservation.state === 'claimed') {
    waiter.close()
    return false
  }
  if (reservation.state === 'pending') {
    await waiter.wait(reservation.expiresAt)
    waiter.close()
    return ringWithIndexedDbClaim(eventKey, kind)
  }
  waiter.close()

  const played = await playCompletionBell(kind)
  if (played) saveLocalClaim(eventKey)
  try {
    await settleIndexedDbClaim(eventKey, owner, played)
  } catch {
    /* best effort; contenders retry after the pending lease expires */
  } finally {
    notifyClaimChanged(eventKey)
  }
  return played
}

async function claimAndRingCompletion(
  eventKey: string,
  kind: CompletionBellKind,
): Promise<boolean> {
  const locks =
    typeof navigator === 'undefined'
      ? undefined
      : (navigator as NavigatorWithLocks).locks
  if (!locks) return ringWithIndexedDbClaim(eventKey, kind)

  try {
    return await locks.request(LOCK_NAME, async () => {
      if (hasActiveLocalClaim(eventKey)) return false
      const played = await playCompletionBell(kind)
      if (played) saveLocalClaim(eventKey)
      return played
    })
  } catch {
    return ringWithIndexedDbClaim(eventKey, kind)
  }
}

/** Claim and ring once for one live top-level terminal status event. */
export async function ringForCompletionEvent(
  event: StatusChangedEvent,
  conversation: BellConversation | undefined,
): Promise<boolean> {
  if (!loadCompletionBellEnabled()) return false
  if (!shouldRingCompletionBell(event, conversation)) return false
  const eventKey = `${event.session_id}:${event.status}:${event.timestamp}`
  return claimAndRingCompletion(
    eventKey,
    event.status === 'error' ? 'failed' : 'completed',
  )
}
