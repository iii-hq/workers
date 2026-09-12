/* iii Console service worker.
 *
 * The Console is an online application: engine state, WebSockets, probes and
 * injected worker UIs must never be answered from an offline cache. Only the
 * application shell and immutable/static presentation assets are retained so
 * an installed Console can open into its normal disconnected state and
 * reconnect when the server becomes reachable again.
 */

const scopeUrl = new URL(self.registration.scope)
const scopePath = scopeUrl.pathname.endsWith('/')
  ? scopeUrl.pathname
  : `${scopeUrl.pathname}/`
const scopeKey = scopePath.replace(/[^a-z0-9]/gi, '-') || 'root'
const cachePrefix = `iii-console-shell-${scopeKey}-`
const cacheName = `${cachePrefix}v2`
const shellUrl = new URL('./', self.registration.scope).href

function relativePath(url) {
  if (url.origin !== scopeUrl.origin || !url.pathname.startsWith(scopePath)) {
    return null
  }
  return url.pathname.slice(scopePath.length)
}

function isStaticAsset(path) {
  return (
    path.startsWith('assets/') ||
    path.startsWith('icons/') ||
    path.startsWith('vendor/') ||
    path === 'manifest.webmanifest'
  )
}

async function cacheApplicationShell() {
  const response = await fetch(shellUrl, { cache: 'no-cache' })
  if (!response.ok) {
    throw new Error(`unable to cache Console shell: HTTP ${response.status}`)
  }

  const html = await response.clone().text()
  const staticUrls = Array.from(
    html.matchAll(/\b(?:href|src)=["']([^"']+)["']/g),
    ([, value]) => new URL(value, shellUrl),
  ).filter((url) => {
    const path = relativePath(url)
    return path !== null && isStaticAsset(path)
  })

  const cache = await caches.open(cacheName)
  await cache.put(shellUrl, response)
  await cache.addAll([...new Set(staticUrls.map((url) => url.href))])
}

self.addEventListener('install', (event) => {
  event.waitUntil(cacheApplicationShell().then(() => self.skipWaiting()))
})

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((names) =>
        Promise.all(
          names
            .filter((name) => name.startsWith(cachePrefix) && name !== cacheName)
            .map((name) => caches.delete(name)),
        ),
      )
      .then(() => self.clients.claim()),
  )
})

self.addEventListener('fetch', (event) => {
  const { request } = event
  if (request.method !== 'GET') return

  const url = new URL(request.url)
  const path = relativePath(url)
  if (path === null) return

  if (request.mode === 'navigate') {
    event.respondWith(
      fetch(request)
        .then((response) => {
          if (
            response.ok &&
            url.href === shellUrl &&
            response.url === shellUrl
          ) {
            const copy = response.clone()
            event.waitUntil(
              caches.open(cacheName).then((cache) => cache.put(shellUrl, copy)),
            )
          }
          return response
        })
        .catch(() => caches.match(shellUrl)),
    )
    return
  }

  if (!isStaticAsset(path)) return

  event.respondWith(
    caches.match(request).then(
      (cached) =>
        cached ??
        fetch(request).then((response) => {
          if (response.ok) {
            const copy = response.clone()
            event.waitUntil(
              caches.open(cacheName).then((cache) => cache.put(request, copy)),
            )
          }
          return response
        }),
    ),
  )
})
