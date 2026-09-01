import { describe, expect, it } from 'vitest'
import { providerCardIds, providerFieldDefinitions } from './provider-cards'

const schema = {
  properties: {
    providers: {
      properties: {
        anthropic: {},
        openai: {},
      },
    },
  },
}

describe('providerCardIds', () => {
  it('shows providers from the schema when fresh-install config is null', () => {
    expect(providerCardIds(schema, null)).toEqual(['anthropic', 'openai'])
  })

  it('keeps configured providers that are no longer in the schema', () => {
    expect(
      providerCardIds(schema, {
        providers: { openai: { api_key: 'configured' }, custom: {} },
      }),
    ).toEqual(['anthropic', 'openai', 'custom'])
  })

  it('returns no cards only when neither source contains providers', () => {
    expect(providerCardIds(null, null)).toEqual([])
    expect(providerCardIds({ properties: {} }, {})).toEqual([])
  })

  it('describes schema-owned scalar fields beyond the default provider fields', () => {
    const customSchema = {
      properties: {
        providers: {
          properties: {
            custom: {
              type: 'object',
              required: ['region'],
              properties: {
                region: {
                  type: 'string',
                  title: 'Region',
                  description: 'Provider region.',
                  enum: ['us-east-1', 'eu-west-1'],
                },
                use_cache: { type: 'boolean' },
                concurrency: { type: 'integer', default: 4 },
                access_token: { type: 'string', writeOnly: true },
              },
            },
          },
        },
      },
    }
    expect(providerFieldDefinitions(customSchema, { providers: { custom: {} } }, 'custom')).toEqual([
      {
        key: 'region',
        label: 'Region',
        description: 'Provider region.',
        kind: 'string',
        writeOnly: false,
        required: true,
        enumValues: ['us-east-1', 'eu-west-1'],
      },
      {
        key: 'use_cache',
        label: 'Use cache',
        kind: 'boolean',
        writeOnly: false,
        required: false,
      },
      {
        key: 'concurrency',
        label: 'Concurrency',
        kind: 'integer',
        writeOnly: false,
        required: false,
        defaultValue: 4,
      },
      {
        key: 'access_token',
        label: 'Access token',
        kind: 'string',
        writeOnly: true,
        required: false,
      },
    ])
  })

  it('keeps unknown configured scalars editable and marks structured values preserved', () => {
    expect(
      providerFieldDefinitions(
        schema,
        {
          providers: {
            openai: {
              api_key: '${' + 'OPENAI_API_KEY}',
              organization: 'acme',
              enabled: true,
              advanced: { mode: 'future' },
            },
          },
        },
        'openai',
      ).map(({ key, kind }) => ({ key, kind })),
    ).toEqual([
      { key: 'api_key', kind: 'string' },
      { key: 'organization', kind: 'string' },
      { key: 'enabled', kind: 'boolean' },
      { key: 'advanced', kind: 'structured' },
    ])
  })
})
