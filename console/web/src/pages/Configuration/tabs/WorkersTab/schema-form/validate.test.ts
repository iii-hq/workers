// biome-ignore-all lint/suspicious/noTemplateCurlyInString: this file's whole point is exercising the literal `${VAR}` env-template syntax — backticks would change what we're testing.
import { describe, expect, it } from 'vitest'
import type { JsonSchema } from '../api'
import {
  coerceScalar,
  resolveLeafForValidation,
  validateConfig,
} from './validate'

describe('coerceScalar', () => {
  it('coerces integers, floats, bools, and null like the engine', () => {
    expect(coerceScalar('8080')).toBe(8080)
    expect(Number.isInteger(coerceScalar('8080') as number)).toBe(true)
    expect(coerceScalar('3.5')).toBe(3.5)
    expect(coerceScalar('true')).toBe(true)
    expect(coerceScalar('false')).toBe(false)
    expect(coerceScalar('null')).toBe(null)
  })

  it('keeps non-scalar-looking text as a string', () => {
    expect(coerceScalar('localhost')).toBe('localhost')
    expect(coerceScalar('127.0.0.1')).toBe('127.0.0.1')
    expect(coerceScalar('on')).toBe('on')
    expect(coerceScalar('')).toBe('')
  })
})

describe('resolveLeafForValidation', () => {
  it('passes through non-strings and plain literals', () => {
    expect(resolveLeafForValidation(42)).toEqual({
      kind: 'value',
      resolved: 42,
    })
    expect(resolveLeafForValidation('hello')).toEqual({
      kind: 'value',
      resolved: 'hello',
    })
  })

  it('coerces the default of a lone placeholder', () => {
    expect(resolveLeafForValidation('${PORT:3111}')).toEqual({
      kind: 'value',
      resolved: 3111,
    })
    expect(resolveLeafForValidation('${FLAG:true}')).toEqual({
      kind: 'value',
      resolved: true,
    })
    expect(resolveLeafForValidation('${NAME:}')).toEqual({
      kind: 'value',
      resolved: '',
    })
  })

  it('reports a defaultless lone placeholder as unresolved', () => {
    expect(resolveLeafForValidation('${PORT}')).toEqual({ kind: 'unresolved' })
  })

  it('keeps embedded templates as strings', () => {
    expect(resolveLeafForValidation('redis://${HOST:h}:6379')).toEqual({
      kind: 'value',
      resolved: 'redis://${HOST:h}:6379',
    })
  })
})

describe('validateConfig — #1916 typed env placeholders', () => {
  const portSchema: JsonSchema = {
    type: 'object',
    required: ['port'],
    properties: {
      port: { type: 'integer', minimum: 1, maximum: 65535 },
    },
  }

  it('accepts a templated integer whose coerced default is valid', () => {
    expect(validateConfig({ port: '${HTTP_PORT:3111}' }, portSchema).size).toBe(
      0,
    )
  })

  it('rejects a templated default out of range', () => {
    const errs = validateConfig({ port: '${HTTP_PORT:70000}' }, portSchema)
    expect(errs.get('/port')).toMatch(/65535/)
  })

  it('rejects a templated default of the wrong type', () => {
    const errs = validateConfig({ port: '${HTTP_PORT:notaport}' }, portSchema)
    expect(errs.get('/port')).toMatch(/integer/)
  })

  it('rejects an out-of-range literal', () => {
    expect(validateConfig({ port: 99999 }, portSchema).get('/port')).toMatch(
      /65535/,
    )
  })

  it('rejects a wrong-typed literal', () => {
    expect(validateConfig({ port: 'nope' }, portSchema).get('/port')).toMatch(
      /integer/,
    )
  })

  it('does not block a defaultless placeholder (resolved at runtime)', () => {
    expect(validateConfig({ port: '${HTTP_PORT}' }, portSchema).size).toBe(0)
  })

  it('flags a missing required property', () => {
    expect(validateConfig({}, portSchema).get('/port')).toMatch(/required/)
  })
})

describe('validateConfig — strings, enums, nullable, dictionaries', () => {
  it('skips pattern on a defaultless templated string but enforces it on a literal', () => {
    const schema: JsonSchema = {
      type: 'object',
      properties: { url: { type: 'string', pattern: '^https://' } },
    }
    expect(validateConfig({ url: '${API_URL}' }, schema).size).toBe(0)
    expect(validateConfig({ url: 'ftp://x' }, schema).get('/url')).toMatch(
      /pattern/,
    )
  })

  it('validates enums, allowing coerced template defaults', () => {
    const schema: JsonSchema = {
      type: 'object',
      properties: { mode: { enum: ['fs', 'bridge'] } },
    }
    expect(validateConfig({ mode: 'fs' }, schema).size).toBe(0)
    expect(validateConfig({ mode: 'nfs' }, schema).get('/mode')).toMatch(
      /one of/,
    )
    expect(validateConfig({ mode: '${ADAPTER:fs}' }, schema).size).toBe(0)
    expect(validateConfig({ mode: '${ADAPTER}' }, schema).size).toBe(0)
  })

  it('handles Option<T> nullable unions', () => {
    const schema: JsonSchema = {
      type: 'object',
      properties: {
        limit: { anyOf: [{ type: 'integer' }, { type: 'null' }] },
      },
    }
    expect(validateConfig({ limit: null }, schema).size).toBe(0)
    expect(validateConfig({ limit: 5 }, schema).size).toBe(0)
    expect(validateConfig({ limit: 'x' }, schema).get('/limit')).toMatch(
      /integer/,
    )
  })

  it('validates dictionary (additionalProperties) values', () => {
    const schema: JsonSchema = {
      type: 'object',
      properties: {
        env: { type: 'object', additionalProperties: { type: 'string' } },
      },
    }
    expect(validateConfig({ env: { A: 'x', B: 'y' } }, schema).size).toBe(0)
    expect(validateConfig({ env: { A: 5 } }, schema).get('/env/A')).toMatch(
      /string/,
    )
  })

  it('points errors at the pointer the form renders (nested + escaped keys)', () => {
    const schema: JsonSchema = {
      type: 'object',
      properties: {
        items: { type: 'array', items: { type: 'integer' } },
        env: { type: 'object', additionalProperties: { type: 'integer' } },
      },
    }
    const errs = validateConfig(
      { items: [1, 'bad'], env: { 'weird/key': 'no' } },
      schema,
    )
    expect(errs.get('/items/1')).toMatch(/integer/)
    expect(errs.get('/env/weird~1key')).toMatch(/integer/)
  })
})

describe('validateConfig — discriminated oneOf (adapter shape)', () => {
  const ROOT: JsonSchema = {
    type: 'object',
    properties: {
      adapter: {
        oneOf: [
          {
            type: 'object',
            properties: {
              name: { enum: ['fs'] },
              config: {
                type: 'object',
                properties: { data_dir: { type: 'string' } },
              },
            },
          },
          {
            type: 'object',
            required: ['config'],
            properties: {
              name: { enum: ['bridge'] },
              config: {
                type: 'object',
                required: ['url'],
                properties: { url: { type: 'string' } },
              },
            },
          },
        ],
      },
    },
  }

  it('validates only the active branch and flags its missing field', () => {
    const errs = validateConfig(
      { adapter: { name: 'bridge', config: {} } },
      ROOT,
    )
    expect(errs.get('/adapter/config/url')).toMatch(/required/)
  })

  it('accepts a valid active branch with a templated leaf', () => {
    expect(
      validateConfig(
        { adapter: { name: 'fs', config: { data_dir: '${DIR:/tmp}' } } },
        ROOT,
      ).size,
    ).toBe(0)
  })
})
