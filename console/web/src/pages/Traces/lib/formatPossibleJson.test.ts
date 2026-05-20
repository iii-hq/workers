import { describe, expect, it } from 'vitest'
import { formatPossibleJson } from './formatPossibleJson'

describe('formatPossibleJson', () => {
  describe('returns formatted JSON', () => {
    it('pretty-prints a JSON object with 2-space indent', () => {
      const result = formatPossibleJson('{"foo":"bar","n":1}')
      expect(result).toBe('{\n  "foo": "bar",\n  "n": 1\n}')
    })

    it('pretty-prints a JSON array', () => {
      const result = formatPossibleJson('[1,2,3]')
      expect(result).toBe('[\n  1,\n  2,\n  3\n]')
    })

    it('handles nested objects with consistent indent', () => {
      const result = formatPossibleJson('{"a":{"b":{"c":1}}}')
      expect(result).toBe(
        '{\n  "a": {\n    "b": {\n      "c": 1\n    }\n  }\n}',
      )
    })

    it('handles a string with leading and trailing whitespace', () => {
      const result = formatPossibleJson('  {"x":1}  ')
      expect(result).toBe('{\n  "x": 1\n}')
    })

    it('handles a typical iii.payload.json payload shape', () => {
      const result = formatPossibleJson(
        '{"message_id":"m-1","data":{"k":"v"},"timestamp":1700000000000}',
      )
      expect(result).toContain('"message_id"')
      expect(result).toContain('"data": {')
      expect(result).toContain('"timestamp": 1700000000000')
    })
  })

  describe('returns null', () => {
    it('returns null for non-string input', () => {
      expect(formatPossibleJson(42)).toBeNull()
      expect(formatPossibleJson(true)).toBeNull()
      expect(formatPossibleJson(null)).toBeNull()
      expect(formatPossibleJson(undefined)).toBeNull()
      expect(formatPossibleJson({ already: 'object' })).toBeNull()
      expect(formatPossibleJson([1, 2, 3])).toBeNull()
    })

    it('returns null for an empty string', () => {
      expect(formatPossibleJson('')).toBeNull()
      expect(formatPossibleJson('   ')).toBeNull()
    })

    it('returns null for a string that does NOT start with { or [', () => {
      expect(formatPossibleJson('hello')).toBeNull()
      expect(formatPossibleJson('42')).toBeNull()
      expect(formatPossibleJson('true')).toBeNull()
      expect(formatPossibleJson('"a quoted string"')).toBeNull()
    })

    it('returns null for malformed JSON (looks like JSON but parse fails)', () => {
      expect(formatPossibleJson('{not valid}')).toBeNull()
      expect(formatPossibleJson('{"unterminated":')).toBeNull()
      expect(formatPossibleJson('[1, 2,')).toBeNull()
    })

    it('returns null for trailing-comma JSON (strict parse)', () => {
      // JSON.parse is strict; trailing commas fail. This is desired —
      // we don't want to silently fix the data, we just want to know if
      // it parses cleanly.
      expect(formatPossibleJson('{"a":1,}')).toBeNull()
    })
  })
})
