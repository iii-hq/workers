import { describe, expect, it } from 'vitest'
import { inlineCodeKind, inlineFileIcon } from './inline-code-kind'

describe('inlineCodeKind', () => {
  it('links a bare web address', () => {
    expect(inlineCodeKind('http://127.0.0.1:3111/link-shortener')).toEqual({
      kind: 'url',
      href: 'http://127.0.0.1:3111/link-shortener',
    })
    expect(inlineCodeKind('https://example.com/a?b=1#c')).toMatchObject({
      kind: 'url',
    })
    expect(inlineCodeKind('curl https://example.com')).toEqual({ kind: 'code' })
    expect(inlineCodeKind('ftp://example.com')).toEqual({ kind: 'code' })
  })

  it('marks file names with their language', () => {
    expect(inlineFileIcon('style.css')).toBe('css')
    expect(inlineFileIcon('src/App.tsx')).toBe('react_ts')
    expect(inlineFileIcon('index.d.ts')).toBe('typescript-def')
    expect(inlineFileIcon('main.rs')).toBe('rust')
    expect(inlineFileIcon('Cargo.toml')).toBe('toml')
    expect(inlineFileIcon('package.json')).toBe('nodejs')
    expect(inlineFileIcon('Dockerfile')).toBe('docker')
    expect(inlineFileIcon('logs/app.log')).toBe('log')
  })

  it('leaves code that only looks like a file name alone', () => {
    expect(inlineFileIcon('console.log')).toBeNull()
    expect(inlineFileIcon('res.json')).toBeNull()
    expect(inlineFileIcon('event.key')).toBeNull()
    expect(inlineFileIcon('foo.bar()')).toBeNull()
    expect(inlineFileIcon('v1.2.3')).toBeNull()
    expect(inlineFileIcon('npm run dev')).toBeNull()
    expect(inlineFileIcon('useState')).toBeNull()
    expect(inlineCodeKind('browser::navigate')).toEqual({ kind: 'code' })
  })
})
