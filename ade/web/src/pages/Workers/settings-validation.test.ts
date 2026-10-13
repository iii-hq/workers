import { describe, expect, it } from 'vitest'
import type { ContainerEntry } from './compose-api'
import { draftFrom, type EnvDraft, rebaseSettingsDraft } from './model'
import { validateSettingsDraft } from './settings-validation'

const entry: ContainerEntry = {
  name: 'http',
  worker: 'path://../http',
  version: null,
  start_after: [],
  env_file: [],
  environment: [
    { key: 'PORT', value: '3000', secret: false },
    { key: 'API_KEY', value: null, secret: true },
  ],
  run: null,
  config_override: 'port: 3000\n',
}

const newVariable = (key = '', value = ''): EnvDraft => ({
  key,
  value,
  secret: false,
  replacing: false,
  removed: false,
  isNew: true,
})

describe('validateSettingsDraft environment', () => {
  it('leaves an unchanged masked secret valid', () => {
    expect(validateSettingsDraft(entry, draftFrom(entry))).toEqual({
      env: {},
      config: null,
      valid: true,
    })
  })

  it.each(['', 'typed value'])(
    'requires a name for a new row with value %j',
    (value) => {
      const draft = draftFrom(entry)
      draft.env.push(newVariable('', value))
      const validation = validateSettingsDraft(entry, draft)
      expect(validation.valid).toBe(false)
      expect(validation.env[2]).toMatch(/name/i)
    },
  )

  it.each(['1PORT', 'API-KEY', 'API.KEY', ' PORT', 'PORT ', 'a b'])(
    'rejects %j without silently sanitizing it',
    (key) => {
      const draft = draftFrom(entry)
      draft.env.push(newVariable(key, 'value'))
      const validation = validateSettingsDraft(entry, draft)
      expect(validation.valid).toBe(false)
      expect(validation.env[2]).toMatch(/letter|underscore/i)
    },
  )

  it('allows a new variable with an empty value and a conventional name', () => {
    const draft = draftFrom(entry)
    draft.env.push(newVariable('_EMPTY_1'))
    expect(validateSettingsDraft(entry, draft).valid).toBe(true)
  })

  it('marks both sides of an existing/new duplicate', () => {
    const draft = draftFrom(entry)
    draft.env.push(newVariable('PORT', '4000'))
    const validation = validateSettingsDraft(entry, draft)
    expect(validation.valid).toBe(false)
    expect(validation.env[0]).toMatch(/duplicate|already/i)
    expect(validation.env[2]).toMatch(/duplicate|already/i)
  })

  it('marks every repeated new key, including three copies', () => {
    const draft = draftFrom(entry)
    draft.env.push(
      newVariable('HOST', 'first'),
      newVariable('HOST', 'second'),
      newVariable('HOST', 'third'),
    )
    const validation = validateSettingsDraft(entry, draft)
    expect(validation.valid).toBe(false)
    expect(Object.keys(validation.env)).toEqual(['2', '3', '4'])
  })

  it('excludes removed keys and blank removed rows from validation', () => {
    const draft = draftFrom(entry)
    draft.env[0].removed = true
    draft.env.push(newVariable('PORT', '4000'), {
      ...newVariable(),
      removed: true,
    })
    expect(validateSettingsDraft(entry, draft).valid).toBe(true)
  })

  it('requires a value or cancellation for pending secret replacement', () => {
    const draft = draftFrom(entry)
    draft.env[1].replacing = true
    const validation = validateSettingsDraft(entry, draft)
    expect(validation.valid).toBe(false)
    expect(validation.env[1]).toMatch(/value.*cancel|cancel.*value/i)
    draft.env[1].value = 'new-secret'
    expect(validateSettingsDraft(entry, draft).valid).toBe(true)
  })

  it('keeps an unfinished secret replacement invalid when readback removed its saved row', () => {
    const draft = draftFrom(entry)
    draft.env[1].replacing = true
    const loaded = { ...entry, environment: [entry.environment[0]] }
    const rebased = rebaseSettingsDraft(entry, draft, loaded)
    expect(rebased.env[1]).toMatchObject({
      isNew: true,
      secret: true,
      replacing: true,
      value: '',
    })
    const validation = validateSettingsDraft(loaded, rebased)
    expect(validation.valid).toBe(false)
    expect(validation.env[1]).toMatch(/value.*cancel|cancel.*value/i)
    rebased.env[1].value = 'new-secret'
    expect(validateSettingsDraft(loaded, rebased).valid).toBe(true)
  })
})

describe('validateSettingsDraft configuration', () => {
  it.each(['', ' \n ', '# optional override\n', '{}'])(
    'accepts an empty override %j',
    (config) => {
      const draft = { ...draftFrom(entry), config }
      expect(validateSettingsDraft(entry, draft).config).toBeNull()
      expect(validateSettingsDraft(entry, draft).valid).toBe(true)
    },
  )

  it.each(['- port\n- host\n', '3000\n', 'null\n', '"port"\n'])(
    'rejects a non-mapping override %j',
    (config) => {
      const validation = validateSettingsDraft(entry, {
        ...draftFrom(entry),
        config,
      })
      expect(validation.valid).toBe(false)
      expect(validation.config).toMatch(/mapping|key.*value/i)
    },
  )

  it('points malformed YAML to a line and column', () => {
    const validation = validateSettingsDraft(entry, {
      ...draftFrom(entry),
      config: 'port: [3000\nhost: localhost\n',
    })
    expect(validation.valid).toBe(false)
    expect(validation.config).toMatch(/line \d+, column \d+/i)
    expect(validation.config).toMatch(/YAML/i)
  })

  it('separates practical list guidance from the original parser detail without losing its position', () => {
    const lines = Array.from(
      { length: 99 },
      (_, index) => `key_${index + 1}: ${index + 1}`,
    )
    const validation = validateSettingsDraft(entry, {
      ...draftFrom(entry),
      config: `${lines.join('\n')}\nbroken: [\n`,
    })
    expect(validation.valid).toBe(false)
    expect(validation.configPosition).toEqual({ line: 101, column: 1 })
    expect(validation.config).toMatch(/line 101, column 1/)
    expect(validation.config).toMatch(/indentation.*close.*\]/i)
    expect(validation.config).not.toMatch(/Flow sequence|block collection/)
    expect(validation).toHaveProperty(
      'configDetail',
      'Flow sequence in block collection must be sufficiently indented and end with a ]',
    )
  })

  it('exposes the exact diagnostic position deep in a long override', () => {
    const lines = Array.from(
      { length: 100 },
      (_, index) => `key_${index + 1}: ${index + 1}`,
    )
    lines[69] = 'key_69: 70'
    const validation = validateSettingsDraft(entry, {
      ...draftFrom(entry),
      config: `${lines.join('\n')}\n`,
    })
    expect(validation.valid).toBe(false)
    expect(validation.config).toMatch(/line 70, column 1/)
    expect(validation.configPosition).toEqual({ line: 70, column: 1 })
  })

  it('does not add a diagnostic position to a valid mapping result', () => {
    const validation = validateSettingsDraft(entry, {
      ...draftFrom(entry),
      config: 'http:\n  host: localhost\n  port: 3000\n',
    })
    expect(validation).toEqual({ env: {}, config: null, valid: true })
    expect(validation).not.toHaveProperty('configPosition')
  })

  it.each(['3000\n', 'http: *missing\n'])(
    'leaves errors without a parser location safe for navigation: %j',
    (config) => {
      const validation = validateSettingsDraft(entry, {
        ...draftFrom(entry),
        config,
      })
      expect(validation.valid).toBe(false)
      expect(validation.config).toBeTruthy()
      expect(validation.configPosition).toBeUndefined()
    },
  )

  it('rejects duplicate mapping keys instead of accepting the last value', () => {
    const validation = validateSettingsDraft(entry, {
      ...draftFrom(entry),
      config: 'port: 3000\nport: 4000\n',
    })
    expect(validation.valid).toBe(false)
    expect(validation.config).toMatch(/unique|duplicate/i)
    expect(validation.config).toMatch(/line 2, column 1/i)
    expect(validation).not.toHaveProperty('configDetail')
  })

  it('allows nested mappings, flow syntax, block strings and YAML merge aliases', () => {
    const config = [
      'defaults: &defaults',
      '  host: localhost',
      'http:',
      '  <<: *defaults',
      '  port: 3000',
      'headers: {accept: application/json}',
      'message: |',
      '  one line',
      '  another line',
      '',
    ].join('\n')
    expect(
      validateSettingsDraft(entry, { ...draftFrom(entry), config }).valid,
    ).toBe(true)
  })

  it('rejects unresolved aliases before the override is applied', () => {
    const validation = validateSettingsDraft(entry, {
      ...draftFrom(entry),
      config: 'http: *missing\n',
    })
    expect(validation.valid).toBe(false)
    expect(validation.config).toMatch(/alias|anchor/i)
  })
})
