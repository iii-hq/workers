import { describe, expect, it } from 'vitest'
import { lastPerKey } from '../src/keys'
import { diffItems, sameJson } from '../src/runtime/diff'
import { parseUrl } from '../src/url'

type Item = { key: string; v?: number }
const key = (i: Item) => i.key
const items = (s: string) => s.split('').map((k) => ({ key: k }))

/** Applies the splices like the host does. */
function apply(prev: Item[], next: Item[]) {
  const out = [...prev]
  for (const [start, del, added] of diffItems(prev, next, key)) out.splice(start, del, ...added)
  return out
}

describe('diffItems', () => {
  it('sends only the changed positions when the keys keep their order', () => {
    const prev = [
      { key: 'a', v: 1 },
      { key: 'b', v: 1 },
      { key: 'c', v: 1 },
      { key: 'd', v: 1 },
    ]
    const next = [
      { key: 'a', v: 1 },
      { key: 'b', v: 2 },
      { key: 'c', v: 2 },
      { key: 'd', v: 1 },
    ]
    expect(diffItems(prev, next, key)).toEqual([[1, 2, [next[1], next[2]]]])
  })

  it('appends, removes and replaces the middle', () => {
    expect(diffItems(items('ab'), items('abc'), key)).toEqual([[2, 0, [{ key: 'c' }]]])
    expect(diffItems(items('abc'), items('ac'), key)).toEqual([[1, 1, []]])
    expect(diffItems(items('abcd'), items('adcb'), key)).toEqual([[1, 3, items('dcb')]])
    expect(diffItems(items('abc'), items('cab'), key)).toEqual([[0, 3, items('cab')]])
    expect(diffItems(items('abc'), items('abc'), key)).toEqual([])
    expect(diffItems([], items('ab'), key)).toEqual([[0, 0, items('ab')]])
    expect(diffItems(items('ab'), [], key)).toEqual([[0, 2, []]])
  })

  it('always lands on the new list', () => {
    const cases = ['', 'a', 'ab', 'ba', 'abc', 'cba', 'bca', 'abcd', 'dabc', 'aXbc', 'xyz']
    for (const p of cases) for (const n of cases) expect(apply(items(p), items(n))).toEqual(items(n))
    const prev = [
      { key: 'a', v: 1 },
      { key: 'b', v: 1 },
      { key: 'c', v: 1 },
    ]
    const next = [{ key: 'a', v: 2 }, { key: 'x' }, { key: 'c', v: 3 }]
    expect(apply(prev, next)).toEqual(next)
  })
})

describe('parseUrl', () => {
  it('splits absolute URLs like URL does', () => {
    expect(parseUrl('https://User@Example.com:8080/a/b?q=1#top')).toEqual({
      protocol: 'https:',
      host: 'example.com:8080',
      pathname: '/a/b',
      search: '?q=1',
      hash: '#top',
    })
    expect(parseUrl('http://iii.dev')).toEqual({
      protocol: 'http:',
      host: 'iii.dev',
      pathname: '/',
      search: '',
      hash: '',
    })
    expect(parseUrl('mailto:a@b.c')).toEqual({ protocol: 'mailto:', host: '', pathname: 'a@b.c', search: '', hash: '' })
    expect(parseUrl('about:blank')?.pathname).toBe('blank')
  })

  it('refuses what has no scheme', () => {
    expect(parseUrl('example.com/path')).toBeNull()
    expect(parseUrl('')).toBeNull()
  })
})

describe('lastPerKey', () => {
  it('keeps the last entry of each key, where it last appears', () => {
    const items = [
      { key: 'a', v: 1 },
      { key: 'b', v: 1 },
      { key: 'a', v: 2 },
      { key: 'c', v: 1 },
      { key: 'b', v: 2 },
    ]
    expect(lastPerKey(items, 'key')).toEqual([
      { key: 'a', v: 2 },
      { key: 'c', v: 1 },
      { key: 'b', v: 2 },
    ])
    expect(lastPerKey(items, (i) => i.v)).toEqual([
      { key: 'c', v: 1 },
      { key: 'b', v: 2 },
    ])
    expect(lastPerKey([], 'key')).toEqual([])
  })
})

describe('sameJson', () => {
  it('compares JSON values as the host reads them', () => {
    const item = { a: 1, b: { c: [1, 'x', null] } }
    expect(sameJson(item, item)).toBe(true)
    expect(sameJson(item, JSON.parse(JSON.stringify(item)))).toBe(true)
    expect(sameJson({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(true)
    expect(sameJson({ a: 1, u: undefined }, { a: 1 })).toBe(true)
    expect(sameJson({ a: 1 }, { a: 1, u: undefined })).toBe(true)
    expect(sameJson({ a: 1 }, { a: 1, b: 2 })).toBe(false)
    expect(sameJson({ a: 1, b: 2 }, { a: 1 })).toBe(false)
    expect(sameJson({ a: [1, 2] }, { a: [1, 2, 3] })).toBe(false)
    expect(sameJson({ a: [1, { b: 2 }] }, { a: [1, { b: 3 }] })).toBe(false)
    expect(sameJson({ a: null }, { a: {} })).toBe(false)
    expect(sameJson({ a: [] }, { a: {} })).toBe(false)
    expect(sameJson({ a: '1' }, { a: 1 })).toBe(false)
    expect(sameJson({ d: new Date(0) }, { d: new Date(0) })).toBe(true)
    expect(sameJson({ d: new Date(0) }, { d: new Date(1) })).toBe(false)
  })
})
