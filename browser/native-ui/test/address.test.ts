/** The address rules (D10: URLs only, the web bar's own toUrl) and how a URL reads on a phone. */
import { describe, expect, it } from 'vitest'
import {
  displayUrl,
  hostOf,
  isBlank,
  isSecure,
  isWebUrl,
  stripControls,
  tabTitle,
  toUrl,
  webAddress,
  webUrl,
} from '../src/address'

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

describe('the web-address gate (D10)', () => {
  it('opens http(s) with a well-formed host, the scheme added by the web rule', () => {
    expect(webAddress('example.com')).toBe('https://example.com')
    expect(webAddress('localhost:3000/x')).toBe('http://localhost:3000/x')
    expect(webAddress('[::1]:8080')).toBe('http://[::1]:8080')
    expect(webAddress('iii.dev@evil.example')).toBe('https://iii.dev@evil.example')
    expect(webAddress('bücher.de')).toBe('https://bücher.de')
  })

  it('refuses every other scheme, a missing or malformed host, and words', () => {
    for (const typed of [
      'javascript://alert(1)',
      'javascript:alert(1)',
      'data:text/html,<b>x</b>',
      'chrome://settings',
      'intent://example.com/#Intent;end',
      'about:config',
      'file:///etc/hosts',
      'ftp://files.example/',
      'view-source:https://a.b',
      'https://',
      'https://?q=1',
      'http://:8080',
      'two words',
      '',
    ]) {
      expect(webAddress(typed), typed).toBeNull()
    }
  })

  it('strips control characters and bidi controls first', () => {
    expect(webAddress('example.com\u0000')).toBe('https://example.com')
    expect(webAddress('exa\u0007mple.com')).toBe('https://example.com')
    expect(webAddress('example.com/\u001b[31mred')).toBe('https://example.com/[31mred')
    expect(stripControls('Ex\u202eample\u0007 Domain\u2066')).toBe('Example Domain')
    expect(stripControls(42)).toBe('')
  })

  it('checks absolute URLs as they are (a history visit), without adding a scheme', () => {
    expect(webUrl('https://example.com/a')).toBe('https://example.com/a')
    expect(webUrl('javascript:alert(1)')).toBeNull()
    expect(webUrl('example.com')).toBeNull()
    expect(webUrl(null)).toBeNull()
  })

  it('names a tab without controls, whatever the title’s type', () => {
    expect(tabTitle('Ex\u202eample\u0007', 'https://example.com/')).toBe('Example')
    expect(tabTitle(42, 'https://odd.example/')).toBe('odd.example')
  })
})
