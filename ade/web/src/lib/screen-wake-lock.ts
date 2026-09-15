/**
 * One screen lock per document, shared by concurrent chats and local work.
 * This only prevents automatic screen sleep while visible; it cannot keep a
 * background PWA running or override the user's power button / OS policy.
 */
export function createScreenWakeLock() {
  let users = 0
  let version = 0
  let pending = false
  let sentinel: WakeLockSentinel | null = null

  const release = (lock: WakeLockSentinel) => {
    // Unsupported/denied/revoked locks must never break the underlying work.
    void lock.release().catch(() => undefined)
  }

  const request = async () => {
    if (
      users === 0 ||
      pending ||
      sentinel ||
      document.visibilityState !== 'visible'
    )
      return
    const requestedVersion = version
    pending = true
    try {
      const lock = await navigator.wakeLock.request('screen')
      if (
        users === 0 ||
        requestedVersion !== version ||
        document.visibilityState !== 'visible'
      ) {
        release(lock)
      } else if (!lock.released) {
        sentinel = lock
        lock.addEventListener(
          'release',
          () => {
            if (sentinel === lock) sentinel = null
            // Do not fight a low-battery/OS revocation in a request loop.
            // Visibility changes or the next user gesture can retry instead.
          },
          { once: true },
        )
      }
    } catch {
      // Best effort: unsupported policy, battery saver, insecure context, etc.
    } finally {
      pending = false
      // A new activity/visibility epoch may have begun during this request.
      if (requestedVersion !== version) void request()
    }
  }

  const drop = () => {
    version += 1
    const lock = sentinel
    sentinel = null
    if (lock) release(lock)
  }
  const onVisibility = () => {
    if (document.visibilityState === 'visible') void request()
    else drop()
  }
  const onInteraction = () => {
    void request()
  }

  return () => {
    if (
      typeof document === 'undefined' ||
      typeof navigator === 'undefined' ||
      !navigator.wakeLock?.request
    )
      return () => undefined

    users += 1
    if (users === 1) {
      version += 1
      document.addEventListener('visibilitychange', onVisibility)
      document.addEventListener('pointerdown', onInteraction)
      document.addEventListener('keydown', onInteraction)
      void request()
    }
    let released = false
    return () => {
      if (released) return
      released = true
      users -= 1
      if (users !== 0) return
      document.removeEventListener('visibilitychange', onVisibility)
      document.removeEventListener('pointerdown', onInteraction)
      document.removeEventListener('keydown', onInteraction)
      drop()
    }
  }
}

/** Acquire a lease; releasing one activity never releases another's lock. */
export const acquireScreenWakeLock = createScreenWakeLock()

/** Cover the whole local operation, including preparation and every exit. */
export function withScreenWakeLock<Args extends unknown[], Result>(
  work: (...args: Args) => Promise<Result>,
): (...args: Args) => Promise<Result> {
  return async (...args) => {
    const release = acquireScreenWakeLock()
    try {
      return await work(...args)
    } finally {
      release()
    }
  }
}
