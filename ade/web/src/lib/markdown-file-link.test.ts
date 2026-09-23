import { describe, expect, it } from 'vitest'
import { parseMarkdownFileLink } from './markdown-file-link'

describe('parseMarkdownFileLink', () => {
  it.each([
    [
      'crates/iii-compose/src/lifecycle.rs#L1437',
      'crates/iii-compose/src/lifecycle.rs',
      1437,
      1437,
    ],
    ['src/a.ts#L12-L40', 'src/a.ts', 12, 40],
    ['src/a.ts#L12-40', 'src/a.ts', 12, 40],
    ['src/a.ts:12–40', 'src/a.ts', 12, 40],
    ['./src/a.ts:40-12', './src/a.ts', 12, 40],
    ['/repo/src/a.ts#L9', '/repo/src/a.ts', 9, 9],
    ['../other/a%20b.ts#L3-L4', '../other/a b.ts', 3, 4],
    ['bin/run#L1', 'bin/run', 1, 1],
  ])('parses %s', (href, path, from, to) => {
    expect(parseMarkdownFileLink(href)).toEqual({ path, range: { from, to } })
  })

  it.each(['README.md', 'Dockerfile', '.env', 'src/a.ts', '/repo/Makefile'])(
    'accepts the file %s',
    (path) => {
      expect(parseMarkdownFileLink(path)).toEqual({ path })
    },
  )

  it.each([
    '',
    '#L10',
    '#section',
    '#/worker/ide',
    '/workers',
    '/docs',
    'src/',
    'https://example.com/a.ts#L2',
    '//example.com/a.ts',
    'mailto:user@example.com',
    'javascript:alert(1)',
    'data:text/plain,x',
    'javascript%3Aalert.ts',
    '%2f%2fexample.com/a.ts',
    'src/%00a.ts',
    'src\\a.ts',
    'src/%zz.ts',
    'src/a.ts?raw=1',
    'src/a.ts#heading',
    'src/a.ts#',
    'src/a.ts#L0',
    'src/a.ts#L2-L0',
    'src/a.ts:0',
    'src/a.ts#L99999999999999999999',
    'src/a.ts:2#L3',
    ' src/a.ts',
  ])('leaves non-file or malformed destination %s alone', (href) => {
    expect(parseMarkdownFileLink(href)).toBeNull()
  })
  it('supports local file URLs, columns and encoded filename delimiters', () => {
    expect(parseMarkdownFileLink('file:///tmp/no-extension#L3')).toEqual({ path: '/tmp/no-extension', range: { from: 3, to: 3 } })
    expect(parseMarkdownFileLink('file://localhost/tmp/a.ts')).toEqual({ path: '/tmp/a.ts' })
    expect(parseMarkdownFileLink('src/a.ts:2:3')).toEqual({ path: 'src/a.ts', range: { from: 2, to: 2 }, column: 3 })
    expect(parseMarkdownFileLink('src/a%23b%3Fc%3A10.ts#L2')).toEqual({ path: 'src/a#b?c:10.ts', range: { from: 2, to: 2 } })
    expect(parseMarkdownFileLink('file://remote/tmp/a.ts')).toBeNull()
    expect(parseMarkdownFileLink('src/a.ts:2:0')).toBeNull()
  })
})
