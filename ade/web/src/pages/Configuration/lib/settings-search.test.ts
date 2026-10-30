import { describe, expect, it } from 'vitest'
import { generalSearchFields } from './general-settings-index'
import {
  buildSettingsSearchIndex,
  highlightRanges,
  humanizeKey,
  type SettingsSearchDoc,
  schemaSearchFields,
  searchSettings,
  sharedIdPrefixes,
  stripIdPrefix,
  suggestSettingsQuery,
} from './settings-search'

const httpSchema = {
  type: 'object',
  properties: {
    port: { type: 'integer', description: 'The port the HTTP listener binds.' },
    host: { type: 'string' },
    default_timeout: { type: 'integer' },
    cors: { $ref: '#/definitions/CorsConfig' },
  },
  definitions: {
    CorsConfig: {
      type: 'object',
      properties: {
        allowed_origins: {
          type: 'array',
          items: { type: 'string' },
          description:
            'Origins that may call the API. Leave empty to allow themselves only.',
        },
      },
    },
  },
}

const adeSchema = {
  type: 'object',
  properties: {
    http_port: { type: 'integer' },
    injectableUi: {
      anyOf: [
        {
          type: 'object',
          properties: { disabled_workers: { type: 'array' } },
        },
        { type: 'null' },
      ],
    },
  },
}

const ids = ['hangzhou-http', 'hangzhou-ade', 'hangzhou-github']
const prefixes = sharedIdPrefixes(ids)

const docs: SettingsSearchDoc[] = [
  {
    id: 'general',
    name: 'General',
    aliases: [],
    description:
      'ADE appearance, permissions, providers, and filesystem access.',
    fields: generalSearchFields(true),
  },
  {
    id: 'hangzhou-http',
    name: 'HTTP',
    aliases: ['http', stripIdPrefix('hangzhou-http', prefixes)],
    description: 'Inbound HTTP server.',
    fields: schemaSearchFields(httpSchema),
  },
  {
    id: 'hangzhou-ade',
    name: 'ADE',
    aliases: ['console', stripIdPrefix('hangzhou-ade', prefixes)],
    description: 'ADE server and UI settings.',
    fields: schemaSearchFields(adeSchema),
  },
  {
    id: 'hangzhou-github',
    name: 'GitHub',
    aliases: ['github'],
    description: 'Repositories, pull requests and webhooks.',
    fields: schemaSearchFields({
      type: 'object',
      properties: { token: { type: 'string' } },
    }),
  },
]

const index = buildSettingsSearchIndex(docs)
const ids_ = (query: string) =>
  searchSettings(index, query).map((r) => r.doc.id)
const fieldLabels = (query: string, docId: string) =>
  searchSettings(index, query)
    .find((r) => r.doc.id === docId)
    ?.fields.map((hit) => hit.field.label)

describe('searchSettings', () => {
  it('returns nothing for an empty query', () => {
    expect(searchSettings(index, '   ')).toEqual([])
  })

  it('finds General rows by label, synonym and enum value', () => {
    expect(ids_('theme')).toEqual(['general'])
    expect(fieldLabels('theme', 'general')).toEqual(['Interface theme'])
    expect(fieldLabels('dark', 'general')).toEqual(['Interface theme'])
    expect(fieldLabels('bell', 'general')).toContain('Completion bell')
  })

  it('matches word starts, never the middle of a word', () => {
    // "port" is inside "support"/"report" in no label, but it must not
    // match "Permanent roots" or "Appearance" either.
    expect(ids_('port')).toEqual(['hangzhou-http', 'hangzhou-ade'])
    expect(ids_('ort')).toEqual([])
  })

  it('ranks a page name above a field on another page', () => {
    expect(ids_('http')[0]).toBe('hangzhou-http')
    expect(ids_('http port')[0]).toBe('hangzhou-ade')
  })

  it('lets one word run across the label’s word breaks', () => {
    expect(ids_('github')).toEqual(['hangzhou-github'])
    expect(ids_('httpport')).toEqual(['hangzhou-ade'])
  })

  it('combines a page name with a field label', () => {
    expect(fieldLabels('github token', 'hangzhou-github')).toEqual(['Token'])
  })

  it('never matches the shared namespace prefix of the ids', () => {
    expect(ids_('hangzhou')).toEqual([])
    expect(ids_('ha')).toEqual([])
  })

  it('matches prose as whole words only', () => {
    // "themselves" in the CORS description must not answer "theme".
    expect(ids_('theme')).toEqual(['general'])
    expect(fieldLabels('origins', 'hangzhou-http')).toEqual(['Allowed origins'])
    expect(fieldLabels('listener', 'hangzhou-http')).toEqual(['Port'])
  })

  it('follows $ref and nullable unions into nested fields', () => {
    const http = schemaSearchFields(httpSchema)
    expect(http.find((f) => f.label === 'Allowed origins')).toMatchObject({
      path: ['cors', 'allowed_origins'],
      trail: ['CORS'],
    })
    const ade = schemaSearchFields(adeSchema)
    expect(ade.find((f) => f.label === 'Disabled workers')).toMatchObject({
      path: ['injectableUi', 'disabled_workers'],
      trail: ['Injectable UI'],
    })
  })

  it('accepts a plural for a singular label', () => {
    expect(fieldLabels('tokens', 'hangzhou-github')).toEqual(['Token'])
  })
})

describe('suggestSettingsQuery', () => {
  it('offers the nearest spelling that has results', () => {
    expect(searchSettings(index, 'githb')).toEqual([])
    expect(suggestSettingsQuery(index, 'githb')).toBe('github')
    expect(suggestSettingsQuery(index, 'thme')).toBe('theme')
  })

  it('offers nothing for a query far from every word', () => {
    expect(suggestSettingsQuery(index, 'xyzzy')).toBeNull()
  })
})

describe('helpers', () => {
  it('humanizes snake and camel keys with acronyms', () => {
    expect(humanizeKey('http_port')).toBe('HTTP port')
    expect(humanizeKey('injectableUi')).toBe('Injectable UI')
    expect(humanizeKey('default_timeout_ms')).toBe('Default timeout ms')
  })

  it('highlights each term where it starts a word', () => {
    expect(highlightRanges('Interface theme', 'the')).toEqual([[10, 13]])
    expect(highlightRanges('GitHub', 'hub')).toEqual([[3, 6]])
    expect(highlightRanges('Default timeout', 'out')).toEqual([])
  })

  it('finds only prefixes shared by two or more ids', () => {
    expect([...sharedIdPrefixes(['a-x', 'a-y', 'iii-observability'])]).toEqual([
      'a-',
    ])
    expect(stripIdPrefix('iii-observability', new Set(['a-']))).toBe(
      'iii-observability',
    )
  })
})
