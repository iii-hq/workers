// biome-ignore-all lint/suspicious/noTemplateCurlyInString: this file's whole point is parsing the literal `${VAR}` syntax — backticks would change what we're testing.
import { describe, expect, it } from 'vitest'
import {
  findFirstPlaceholder,
  parseTemplate,
  serializePlaceholder,
  serializeSegments,
} from './env-template'

describe('parseTemplate', () => {
  it('returns an empty list for empty input', () => {
    expect(parseTemplate('')).toEqual([])
  })

  it('returns a single text segment for plain text', () => {
    expect(parseTemplate('hello world')).toEqual([
      { kind: 'text', text: 'hello world' },
    ])
  })

  it('parses a bare placeholder without default', () => {
    expect(parseTemplate('${PGUSER}')).toEqual([
      { kind: 'placeholder', variable: 'PGUSER', defaultValue: null },
    ])
  })

  it('parses a placeholder with default', () => {
    expect(parseTemplate('${PGUSER:admin}')).toEqual([
      { kind: 'placeholder', variable: 'PGUSER', defaultValue: 'admin' },
    ])
  })

  it('keeps explicit empty-string defaults distinct from missing defaults', () => {
    const segments = parseTemplate('${PGUSER:}')
    expect(segments).toEqual([
      { kind: 'placeholder', variable: 'PGUSER', defaultValue: '' },
    ])
  })

  it('parses mixed text and placeholders in order', () => {
    expect(
      parseTemplate('postgres://${PGUSER:admin}@${PGHOST:localhost}/${PGDB}'),
    ).toEqual([
      { kind: 'text', text: 'postgres://' },
      { kind: 'placeholder', variable: 'PGUSER', defaultValue: 'admin' },
      { kind: 'text', text: '@' },
      { kind: 'placeholder', variable: 'PGHOST', defaultValue: 'localhost' },
      { kind: 'text', text: '/' },
      { kind: 'placeholder', variable: 'PGDB', defaultValue: null },
    ])
  })

  it('leaves unterminated placeholders as literal text', () => {
    expect(parseTemplate('hello ${PGUSER')).toEqual([
      { kind: 'text', text: 'hello ${PGUSER' },
    ])
  })

  it('does not treat `$VAR` (no braces) as a placeholder', () => {
    expect(parseTemplate('$PGUSER')).toEqual([
      { kind: 'text', text: '$PGUSER' },
    ])
  })

  it('accepts permissive variable names (engine-style)', () => {
    expect(parseTemplate('${some.var-name}')).toEqual([
      { kind: 'placeholder', variable: 'some.var-name', defaultValue: null },
    ])
  })

  it('default values may contain colons after the first one', () => {
    expect(parseTemplate('${URL:postgres://localhost:5432}')).toEqual([
      {
        kind: 'placeholder',
        variable: 'URL',
        defaultValue: 'postgres://localhost:5432',
      },
    ])
  })
})

describe('serializeSegments / serializePlaceholder', () => {
  it('round-trips a complex template', () => {
    const input = 'postgres://${PGUSER:admin}@${PGHOST}/${PGDB:my_db}'
    expect(serializeSegments(parseTemplate(input))).toBe(input)
  })

  it('emits `${VAR}` for null defaults', () => {
    expect(
      serializePlaceholder({
        kind: 'placeholder',
        variable: 'PGUSER',
        defaultValue: null,
      }),
    ).toBe('${PGUSER}')
  })

  it('emits `${VAR:}` for explicit empty-string defaults', () => {
    expect(
      serializePlaceholder({
        kind: 'placeholder',
        variable: 'PGUSER',
        defaultValue: '',
      }),
    ).toBe('${PGUSER:}')
  })

  it('emits `${VAR:default}` for non-empty defaults', () => {
    expect(
      serializePlaceholder({
        kind: 'placeholder',
        variable: 'PGUSER',
        defaultValue: 'admin',
      }),
    ).toBe('${PGUSER:admin}')
  })
})

describe('findFirstPlaceholder', () => {
  it('returns null for plain text', () => {
    expect(findFirstPlaceholder('hello world')).toBeNull()
  })

  it('finds the first match at the start of the string', () => {
    const m = findFirstPlaceholder('${VAR} rest')
    expect(m).not.toBeNull()
    expect(m?.start).toBe(0)
    expect(m?.end).toBe(6)
    expect(m?.segment.variable).toBe('VAR')
  })

  it('finds a match in the middle', () => {
    const m = findFirstPlaceholder('prefix ${VAR:def} suffix')
    expect(m?.start).toBe(7)
    expect(m?.end).toBe(17)
    expect(m?.segment.defaultValue).toBe('def')
  })

  it('honors an offset, skipping earlier matches', () => {
    const input = '${A} ${B}'
    const m = findFirstPlaceholder(input, 4)
    expect(m?.start).toBe(5)
    expect(m?.segment.variable).toBe('B')
  })

  it('returns null when no match is found after the offset', () => {
    expect(findFirstPlaceholder('${VAR} rest', 6)).toBeNull()
  })
})
