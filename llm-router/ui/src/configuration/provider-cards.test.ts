import { describe, expect, it } from 'vitest'
import { providerCardIds } from './provider-cards'

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
})
