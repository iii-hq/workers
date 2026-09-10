/**
 * `crypto.randomUUID` polyfill for insecure contexts.
 *
 * The API only exists in secure contexts (https / localhost), and the
 * console is routinely opened over plain http on a LAN IP
 * (`http://192.168.x.x:3113`), where dependencies that call it bare —
 * e.g. iii-browser-sdk ≤ 0.21.6 minting invocation ids — throw and the
 * page renders with no data. `crypto.getRandomValues` IS available in
 * insecure contexts, so back-fill a spec-shaped UUIDv4.
 *
 * Self-installs on import AND is called explicitly by `main.tsx` as its
 * first import — the named-import call keeps the module alive even if
 * `"sideEffects": false` is ever added to package.json, and both paths
 * run before any SDK code can touch `crypto.randomUUID`.
 */

function uuidV4(): `${string}-${string}-${string}-${string}-${string}` {
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  bytes[6] = (bytes[6] & 0x0f) | 0x40 // version 4
  bytes[8] = (bytes[8] & 0x3f) | 0x80 // RFC 4122 variant
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

export function installRandomUUIDPolyfill(): void {
  if (typeof crypto === 'undefined') return
  if (typeof crypto.randomUUID === 'function') return
  // No CSPRNG → don't install. A randomUUID that throws would defeat every
  // `typeof crypto.randomUUID === 'function'` guard in the app (session-id
  // mintId, iii-client makeBrowserId, …) and turn their working fallbacks
  // into a first-render crash on crypto-stripped WebViews.
  if (typeof crypto.getRandomValues !== 'function') {
    console.warn(
      '[crypto-polyfill] crypto.getRandomValues unavailable — randomUUID not installed; guarded fallbacks stay active',
    )
    return
  }
  try {
    // defineProperty rather than assignment: `crypto` can carry
    // non-writable properties, and a failed install must never crash boot
    // — guarded call sites still have their own fallbacks.
    Object.defineProperty(crypto, 'randomUUID', {
      value: uuidV4,
      configurable: true,
      writable: true,
    })
  } catch (err) {
    // Leave the API missing; only unguarded third-party calls lose out —
    // but say so, or a LAN no-data report is undiagnosable from the console.
    console.warn('[crypto-polyfill] install failed', err)
  }
}

installRandomUUIDPolyfill()
