/** The address rules (D10: URLs only, the web bar's own toUrl) and how a URL reads on a phone. */
import { describe, expect, it } from 'vitest'
import { displayUrl, hostOf, isBlank, isSecure, isWebUrl, tabTitle, toUrl } from '../src/address'

describe('toUrl (shared with the console UI)', () => {
  it('keeps a scheme, sends local hosts over http and the rest over https, never searches', () => {
    expect(toUrl('https://example.com/a')).toBe('https://example.com/a')
    expect(toUrl('about:blank')).toBe('about:blank')
    expect(toUrl('localhost:5173')).toBe('http://localhost:5173')
    expect(toUrl('10.0.0.2/admin')).toBe('http://10.0.0.2/admin')
    expect(toUrl('example.com')).toBe('https://example.com')
    expect(toUrl('two words')).toBe('https://two words')
    expect(toUrl('   ')).toBe('')
  })
})

describe('reading a URL', () => {
  it('drops the http(s) scheme and blank pages', () => {
    expect(displayUrl('https://example.com/')).toBe('example.com/')
    expect(displayUrl('http://localhost:3000/x?y=1')).toBe('localhost:3000/x?y=1')
    expect(displayUrl('about:blank')).toBe('')
    expect(displayUrl('chrome-error://chromewebdata/')).toBe('chrome-error://chromewebdata/')
  })

  it('takes the host without credentials, lower case', () => {
    expect(hostOf('https://user:pw@Example.COM:8443/a')).toBe('example.com:8443')
    expect(hostOf('about:blank')).toBe('')
    expect(hostOf('not a url')).toBe('')
  })

  it('knows https, web URLs and blank tabs', () => {
    expect(isSecure('https://a.b')).toBe(true)
    expect(isSecure('http://a.b')).toBe(false)
    expect(isWebUrl('http://a.b')).toBe(true)
    expect(isWebUrl('file:///etc')).toBe(false)
    expect(isBlank('')).toBe(true)
    expect(isBlank('about:blank')).toBe(true)
  })

  it('names a tab by title, host, or New tab', () => {
    expect(tabTitle('Example Domain', 'https://example.com/')).toBe('Example Domain')
    expect(tabTitle('', 'https://example.com/')).toBe('example.com')
    expect(tabTitle('https://example.com/', 'https://example.com/')).toBe('example.com')
    expect(tabTitle('about:blank', 'about:blank')).toBe('New tab')
    expect(tabTitle(undefined, '')).toBe('New tab')
  })
})
