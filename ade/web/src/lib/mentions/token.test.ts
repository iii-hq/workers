import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import {
  findMentions,
  formatMention,
  isValidMentionName,
  parseMentions,
  parseProseMentions,
  uniqueMentions,
} from './token'

interface Fixtures {
  parse: Array<{
    name: string
    text: string
    prose?: boolean
    mentions: Array<{ name: string; id: string; token: string }>
  }>
  format: Array<{ name: string; id: string; token: string }>
}

/* The grammar fixtures the Rust parser (crates/mention-contract) runs too. */
const fixtures: Fixtures = JSON.parse(
  readFileSync(
    new URL(
      '../../../../../crates/mention-contract/fixtures/tokens.json',
      import.meta.url,
    ),
    'utf8',
  ),
)

describe('mention token grammar (shared fixtures)', () => {
  for (const c of fixtures.parse) {
    it(`parses: ${c.name}`, () => {
      const got = c.prose ? parseProseMentions(c.text) : parseMentions(c.text)
      expect(got).toEqual(c.mentions)
    })
  }

  for (const c of fixtures.format) {
    it(`formats ${JSON.stringify(c.id)}`, () => {
      expect(formatMention(c.name, c.id)).toBe(c.token)
      const [parsed] = parseMentions(c.token)
      expect(parsed).toMatchObject({ name: c.name, id: c.id })
    })
  }
})

describe('mention token helpers', () => {
  it('bounds the id and the name length', () => {
    expect(parseMentions(formatMention('x', 'a'.repeat(512)))).toHaveLength(1)
    expect(parseMentions(formatMention('x', 'a'.repeat(513)))).toHaveLength(0)
    expect(isValidMentionName('a'.repeat(40))).toBe(true)
    expect(isValidMentionName('a'.repeat(41))).toBe(false)
    expect(isValidMentionName('fn')).toBe(false)
  })

  it('reports UTF-16 offsets for splitting rendered text', () => {
    const text = 'é @kanban(id="1") and @trace(id="2")'
    const found = findMentions(text)
    expect(found.map((m) => m.index)).toEqual([2, 22])
    expect(
      text.slice(found[1].index, found[1].index + found[1].token.length),
    ).toBe('@trace(id="2")')
  })

  it('keeps the first occurrence of each item', () => {
    const refs = parseMentions(
      '@kanban(id="1") @kanban(id="\\u0031") @session(id="1")',
    )
    expect(uniqueMentions(refs).map((r) => `${r.name}:${r.id}`)).toEqual([
      'kanban:1',
      'session:1',
    ])
  })
})
