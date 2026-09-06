import { describe, expect, it } from 'vitest'
import { isLocalHost, toUrl } from '../lib/address'

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
})
