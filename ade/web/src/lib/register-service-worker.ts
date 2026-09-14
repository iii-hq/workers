export function registerServiceWorker(): void {
  if (!import.meta.env.PROD || !('serviceWorker' in navigator)) return

  const register = () => {
    void navigator.serviceWorker
      .register(new URL('./sw.js', document.baseURI))
      .catch((error: unknown) => {
        // A failed registration must not stop the Console from running as a
        // normal web app (for example, when served from an insecure LAN URL).
        console.warn('[iii-pwa] service worker registration failed', error)
      })
  }

  if (document.readyState === 'complete') {
    register()
  } else {
    window.addEventListener('load', register, { once: true })
  }
}
