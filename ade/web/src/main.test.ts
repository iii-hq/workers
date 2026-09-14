import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

/**
 * The crypto polyfill must evaluate before the SDK-bearing module graph
 * (`./App` → iii-client → iii-browser-sdk) so `crypto.randomUUID` exists
 * before any SDK code touches it — on insecure origins (http://<LAN-IP>)
 * the native API is missing and unguarded callers throw. Every vitest run
 * happens under Node where the API exists, so only these source checks
 * can catch the wiring being dropped or demoted; the runtime symptom
 * (blank data on LAN access) never shows up in unit tests.
 */
describe('main.tsx polyfill wiring', () => {
  const src = readFileSync(new URL('./main.tsx', import.meta.url), 'utf8')

  it('imports the crypto polyfill before the App module graph', () => {
    const polyfillAt = src.indexOf("from '@/lib/crypto-polyfill'")
    const appAt = src.indexOf("from './App'")
    expect(polyfillAt).toBeGreaterThan(-1)
    expect(appAt).toBeGreaterThan(-1)
    expect(polyfillAt).toBeLessThan(appAt)
  })

  it('calls the polyfill installer explicitly (tree-shake proof)', () => {
    expect(src).toContain('installRandomUUIDPolyfill()')
  })
})

describe('main.tsx injectable UI readiness wiring', () => {
  const src = readFileSync(new URL('./main.tsx', import.meta.url), 'utf8')

  it('marks assets as loading before asynchronous client bootstrap', () => {
    const loadingAt = src.indexOf("setUiAssetsStatus('loading')")
    const clientBootstrapAt = src.indexOf('getIiiClient()', loadingAt)

    expect(loadingAt).toBeGreaterThan(-1)
    expect(clientBootstrapAt).toBeGreaterThan(-1)
    expect(loadingAt).toBeLessThan(clientBootstrapAt)
  })

  it('allows built-in forms to recover when client bootstrap fails', () => {
    expect(src).toContain("setUiAssetsStatus('unavailable')")
  })
})

describe('main.tsx official favicon wiring', () => {
  const src = readFileSync(new URL('./main.tsx', import.meta.url), 'utf8')
  const icon = readFileSync(
    new URL('../public/icons/icon.svg', import.meta.url),
    'utf8',
  )
  const manifest = JSON.parse(
    readFileSync(
      new URL('../public/manifest.webmanifest', import.meta.url),
      'utf8',
    ),
  ) as { icons: Array<{ src: string }> }

  it('uses the canonical iii.dev six-bar SVG geometry', () => {
    expect(icon).toContain('viewBox="0 0 933.61 1050.31"')
    expect(icon.match(/<rect class="bar"/g)).toHaveLength(6)
    expect(icon).toContain('x="350.1"')
    expect(icon).toContain('x="700.21"')
    expect(src).toContain("new URL('./icons/icon.svg', document.baseURI)")
    expect(src).not.toContain('./icons/favicon.svg?url')
  })

  it('references only the versioned official PWA icon fallbacks', () => {
    expect(manifest.icons.map(({ src: iconSrc }) => iconSrc)).toEqual([
      './icons/icon.svg',
      './icons/iii-192.png',
      './icons/iii-512.png',
      './icons/iii-maskable-512.png',
    ])
  })
})
