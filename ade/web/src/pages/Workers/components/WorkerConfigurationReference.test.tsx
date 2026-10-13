// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { JsonSchema } from '@/pages/Configuration/tabs/WorkersTab/api'
import { WorkerConfigurationReference } from './WorkerConfigurationReference'
import { loadWorkerConfigurationReference } from './worker-configuration-reference'

vi.mock('./worker-configuration-reference', () => ({
  loadWorkerConfigurationReference: vi.fn(),
}))

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  vi.resetAllMocks()
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
})

async function showSchema(schema: JsonSchema) {
  vi.mocked(loadWorkerConfigurationReference).mockResolvedValue({
    kind: 'schema',
    entry: {
      id: 'registered-test',
      name: 'Registered test',
      description: 'Registered metadata fixture',
      schema,
    },
  })
  await act(async () => {
    root.render(<WorkerConfigurationReference name="test" />)
  })
  await act(async () => {
    const details = container.querySelector('details')
    if (!details) throw new Error('Configuration reference was not rendered')
    details.open = true
    details.dispatchEvent(new Event('toggle'))
  })
}

function fieldText(key: string) {
  const row = [...container.querySelectorAll('[role="listitem"]')].find(
    (item) => item.querySelector('code')?.textContent === key,
  )
  if (!row) throw new Error(`No registered field row for ${key}`)
  return row.textContent ?? ''
}

describe('registered schema metadata', () => {
  it('shows declared bounds and lengths, preserving zero and exclusive limits', async () => {
    await showSchema({
      type: 'object',
      properties: {
        http_port: { type: 'integer', minimum: 0, maximum: 65535 },
        ratio: { type: 'number', exclusiveMinimum: -1, exclusiveMaximum: 1 },
        text: { type: 'string', minLength: 0, maxLength: 12 },
      },
    })
    expect(fieldText('http_port')).toContain('Minimum: 0')
    expect(fieldText('http_port')).toContain('Maximum: 65535')
    expect(fieldText('ratio')).toContain('Minimum (exclusive): -1')
    expect(fieldText('ratio')).toContain('Maximum (exclusive): 1')
    expect(fieldText('text')).toContain('Min length: 0')
    expect(fieldText('text')).toContain('Max length: 12')
  })

  it('keeps actual enum values and declared falsy defaults without inventing secret values', async () => {
    await showSchema({
      type: 'object',
      required: ['mode'],
      properties: {
        mode: {
          enum: ['off', 'coerce', 'judge', 0, false, null, ''],
          default: null,
        },
        count: { type: 'integer', default: 0 },
        enabled: { type: 'boolean', default: false },
        api_token: { type: 'string', writeOnly: true },
      },
    })
    expect(fieldText('mode')).toContain(
      'Allowed values: ["off","coerce","judge",0,false,null,""]',
    )
    expect(fieldText('mode')).toContain('Required in registered schema')
    expect(fieldText('mode')).toContain('Registered default: null')
    expect(fieldText('count')).toContain('Registered default: 0')
    expect(fieldText('enabled')).toContain('Registered default: false')
    expect(fieldText('api_token')).toContain('No declared default')
    expect(fieldText('api_token')).not.toMatch(
      /Required|Allowed values|length:/,
    )
    expect(container.querySelector('input')).toBeNull()
  })

  it('resolves registered references while leaving invalid and composed constraints to the complete schema', async () => {
    await showSchema({
      type: 'object',
      allOf: [{ required: ['invalid'] }],
      definitions: { Port: { type: 'integer', minimum: 0, maximum: 65535 } },
      properties: {
        direct: { $ref: '#/definitions/Port', minimum: 7 },
        wrapped: { allOf: [{ $ref: '#/definitions/Port' }], maximum: 42 },
        invalid: {
          type: 'string',
          minimum: Number.NaN,
          maximum: Number.POSITIVE_INFINITY,
          exclusiveMinimum: true,
          exclusiveMaximum: '10',
          minLength: -1,
          maxLength: 1.5,
          enum: 'off,coerce,judge',
        },
        inferred: { type: 'integer', format: 'uint16' },
        composed: { allOf: [{ minimum: 1 }, { maximum: 9 }] },
        nested: {
          type: 'object',
          properties: { child: { type: 'string', minLength: 2 } },
          required: ['child'],
        },
      },
    })
    expect(fieldText('direct')).toContain('Minimum: 0')
    expect(fieldText('direct')).not.toContain('Minimum: 7')
    expect(fieldText('wrapped')).toContain('Minimum: 0')
    expect(fieldText('wrapped')).toContain('Maximum: 42')
    for (const key of ['invalid', 'inferred', 'composed', 'nested']) {
      expect(fieldText(key)).not.toMatch(
        /Minimum|Maximum|length:|Allowed values|Required in registered schema/,
      )
    }
    expect(
      container.querySelector('.wk-config-reference-complete pre')?.textContent,
    ).toContain('"minLength": 2')
  })
})
