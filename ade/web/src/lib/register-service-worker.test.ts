import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const source = readFileSync(
  new URL('./register-service-worker.ts', import.meta.url),
  'utf8',
)

describe('service worker registration', () => {
  it('is enabled only in production on supported browsers', () => {
    expect(source).toContain('import.meta.env.PROD')
    expect(source).toContain("'serviceWorker' in navigator")
  })

  it('resolves the worker relative to the mounted Console path', () => {
    expect(source).toContain("new URL('./sw.js', document.baseURI)")
  })

  it('does not make registration failure fatal', () => {
    expect(source).toContain('.catch(')
  })

  it('pre-caches the compiled application shell on installation', () => {
    const worker = readFileSync(
      new URL('../../public/sw.js', import.meta.url),
      'utf8',
    )
    expect(worker).toContain('html.matchAll')
    expect(worker).toContain('cache.addAll')
  })
})
