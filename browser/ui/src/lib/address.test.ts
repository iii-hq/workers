import { describe, expect, it } from 'vitest'
import { displayUrl, isLocalHost, siteSecurity, toUrl, toUrlOrSearch } from '../lib/address'

describe('address bar url guessing', () => {
  it('reaches local hosts over http and everything else over https', () => {
    expect(toUrl('localhost:3000')).toBe('http://localhost:3000')
    expect(toUrl('app.localhost/x')).toBe('http://app.localhost/x')
    expect(toUrl('127.0.0.1:8080/a?b')).toBe('http://127.0.0.1:8080/a?b')
    expect(toUrl('192.168.1.20')).toBe('http://192.168.1.20')
    expect(toUrl('example.com')).toBe('https://example.com')
    expect(toUrl('http://example.com')).toBe('http://example.com')
    expect(toUrl('  ')).toBe('')
    expect(isLocalHost('172.20.0.1')).toBe(true)
    expect(isLocalHost('172.32.0.1')).toBe(false)
  })

  it('searches what is not an address', () => {
    expect(toUrlOrSearch('react hooks')).toBe('https://www.google.com/search?q=react%20hooks')
    expect(toUrlOrSearch('github')).toBe('https://www.google.com/search?q=github')
    expect(toUrlOrSearch('localhost')).toBe('http://localhost')
    expect(toUrlOrSearch('intranet:8080/x')).toBe('https://intranet:8080/x')
    expect(toUrlOrSearch('docs.rs/serde')).toBe('https://docs.rs/serde')
  })

  it('reads a page url the way Chrome shows it', () => {
    expect(displayUrl('https://github.com/')).toEqual({ host: 'github.com', rest: '' })
    expect(displayUrl('https://github.com/iii-hq/workers?tab=1#x')).toEqual({
      host: 'github.com',
      rest: '/iii-hq/workers?tab=1#x',
    })
    expect(displayUrl('http://localhost:5173/')).toEqual({ host: 'localhost:5173', rest: '' })
    expect(displayUrl('file:///tmp/a.pdf')).toEqual({ host: '', rest: 'file:///tmp/a.pdf' })
  })

  it('says whether the connection is secure', () => {
    expect(siteSecurity('https://a.test/')).toBe('secure')
    expect(siteSecurity('http://a.test/')).toBe('insecure')
    expect(siteSecurity('http://localhost:3000/')).toBe('local')
    expect(siteSecurity('file:///tmp/a.pdf')).toBe('file')
    expect(siteSecurity('')).toBe('none')
    expect(siteSecurity('about:blank')).toBe('none')
  })
})
