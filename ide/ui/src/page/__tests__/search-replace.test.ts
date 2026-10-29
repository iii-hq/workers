import { describe, expect, it, vi } from 'vitest'
import type { Host } from '@iii-dev/console-ui'
import {
  applyCase,
  expandTemplate,
  previewReplacement,
  replaceAllInText,
  replaceInFile,
  replaceOneInText,
  replaceSummary,
  type ReplaceOptions,
} from '../search-replace'

const opts = (overrides: Partial<ReplaceOptions>): ReplaceOptions => ({
  query: 'foo',
  regex: false,
  ignoreCase: true,
  wholeWord: false,
  replacement: 'bar',
  preserveCase: false,
  ...overrides,
})

describe('replaceAllInText', () => {
  it('replaces every literal match and counts them', () => {
    expect(replaceAllInText('foo food\nFOO', opts({}))).toEqual({ content: 'bar bard\nbar', count: 3 })
  })

  it('honours match case and whole word', () => {
    expect(replaceAllInText('foo Foo food', opts({ ignoreCase: false, wholeWord: true }))).toEqual({
      content: 'bar Foo food',
      count: 1,
    })
  })

  it('takes a literal replacement as typed, `$` included', () => {
    expect(replaceAllInText('a.b', opts({ query: '.', replacement: '$&$1' })).content).toBe('a$&$1b')
  })

  it('expands groups in regex mode, with the surrounding context', () => {
    const out = replaceAllInText('let x = 1\nlet y = 2', opts({ query: '^let (\\w+)', regex: true, replacement: 'const $1' }))
    expect(out).toEqual({ content: 'const x = 1\nconst y = 2', count: 2 })
    expect(replaceAllInText('ab', opts({ query: '(?<=a)b', regex: true, replacement: 'c' })).content).toBe('ac')
  })

  it('preserves the case of each match', () => {
    expect(replaceAllInText('foo Foo FOO', opts({ preserveCase: true })).content).toBe('bar Bar BAR')
  })

  it('leaves the text alone for an unfinished regex', () => {
    expect(replaceAllInText('foo(', opts({ query: '(', regex: true }))).toEqual({ content: 'foo(', count: 0 })
  })
})

describe('replaceOneInText', () => {
  it('replaces only the match at the reported byte column', () => {
    expect(replaceOneInText('x foo foo\nfoo', 1, 7, opts({}))).toEqual({ content: 'x foo bar\nfoo', count: 1 })
  })

  it('counts columns in UTF-8 bytes', () => {
    // `é` is two bytes: the second foo starts at byte 8, character 7.
    expect(replaceOneInText('é foo foo', 1, 8, opts({})).content).toBe('é foo bar')
  })

  it('falls back to the first match on the line and keeps CRLF endings', () => {
    expect(replaceOneInText('a\r\nfoo foo\r\n', 2, 99, opts({})).content).toBe('a\r\nbar foo\r\n')
  })

  it('does nothing past the end of the file', () => {
    expect(replaceOneInText('foo', 3, 1, opts({})).count).toBe(0)
  })
})

describe('helpers', () => {
  it('cases a replacement like the match', () => {
    expect(applyCase('Hello', 'world')).toBe('World')
    expect(applyCase('HELLO', 'world')).toBe('WORLD')
    expect(applyCase('hello', 'World')).toBe('world')
    expect(applyCase('123', 'World')).toBe('World')
  })

  it('expands templates', () => {
    const m = ['ab', 'a', 'b']
    expect(expandTemplate('$2$1 $$ $& $9 $12', m)).toBe('ba $ ab $9 a2')
    expect(expandTemplate('$<x>!', ['a', 'a'], { x: 'a' })).toBe('a!')
  })

  it('previews a hit the way the replace will write it', () => {
    expect(previewReplacement('Foo', opts({ preserveCase: true }))).toBe('Bar')
    expect(previewReplacement('let x', opts({ query: 'let (\\w)', regex: true, replacement: '$1!' }))).toBe('x!')
  })

  it('summarizes a run', () => {
    expect(
      replaceSummary([
        { path: '/a', status: 'replaced', count: 2 },
        { path: '/b', status: 'replaced', count: 1 },
        { path: '/c', status: 'skipped', reason: 'has unsaved edits' },
      ]),
    ).toBe('Replaced 3 occurrences in 2 files · 1 skipped (has unsaved edits)')
    expect(replaceSummary([{ path: '/a', status: 'unchanged' }])).toBe('Nothing was replaced')
  })
})

describe('replaceInFile', () => {
  const hostWith = (read: object, write: object = { success: true, revision: 'r2' }) => {
    const trigger = vi.fn(async (id: string) => {
      if (id === 'coder::read-file') return read
      return { results: [{ path: '/f', bytes_written: 1, ...write }] }
    })
    return { host: { iii: { trigger } } as unknown as Host, trigger }
  }

  it('writes the rewritten body against the revision it read', async () => {
    const { host, trigger } = hostWith({ content: 'foo', revision: 'r1', mode: 0o644, is_utf8: true })
    const out = await replaceInFile(host, '/f', (text) => replaceAllInText(text, opts({})))
    expect(out).toEqual({ path: '/f', status: 'replaced', count: 1 })
    expect(trigger).toHaveBeenLastCalledWith('coder::create-file', {
      files: [{ path: '/f', content: 'bar', overwrite: true, mode: '0644', expected_revision: 'r1' }],
    })
  })

  it('skips a partial read and a binary file without writing', async () => {
    const partial = hostWith({ content: 'foo', more_lines: true })
    expect((await replaceInFile(partial.host, '/f', (t) => replaceAllInText(t, opts({})))).status).toBe('skipped')
    const binary = hostWith({ content: '', is_utf8: false })
    expect((await replaceInFile(binary.host, '/f', (t) => replaceAllInText(t, opts({})))).status).toBe('skipped')
    expect(partial.trigger).toHaveBeenCalledTimes(1)
    expect(binary.trigger).toHaveBeenCalledTimes(1)
  })

  it('reports a revision conflict as skipped', async () => {
    const { host } = hostWith({ content: 'foo', revision: 'r1' }, { success: false, error: { code: 'conflict', message: 'changed on disk' } })
    expect(await replaceInFile(host, '/f', (t) => replaceAllInText(t, opts({})))).toEqual({
      path: '/f',
      status: 'skipped',
      reason: 'changed on disk',
    })
  })
})
