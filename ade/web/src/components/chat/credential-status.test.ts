import { describe, expect, it } from 'vitest'
import type { ProviderListEntry } from '@/lib/models-catalog'
import { credentialStatusFor } from './ProviderConfigurationPanel'

const anthropic: ProviderListEntry = {
  id: 'anthropic',
  display_name: 'Anthropic',
  supports_model_listing: true,
  credential_env_var: 'ANTHROPIC_API_KEY',
  configured: true,
  available: true,
  credential_source: 'secret',
}

describe('credentialStatusFor', () => {
  it('counts a resolved key with models as connected, with the count', () => {
    expect(credentialStatusFor(anthropic, { phase: 'idle' }, 11)).toMatchObject(
      {
        connected: true,
        detail: '11 models',
        error: undefined,
      },
    )
  })

  it('treats a resolved key that lists no models as refused, not connected', () => {
    const status = credentialStatusFor(
      anthropic,
      { phase: 'done', result: { configured: true, models: 0 } },
      0,
    )
    expect(status.connected).toBe(false)
    expect(status.error).toMatch(/lists no models with this key/)
  })

  it('says nothing about models while the key is being checked', () => {
    const status = credentialStatusFor(anthropic, { phase: 'checking' }, 0)
    expect(status.checking).toBe(true)
    expect(status.error).toBeUndefined()
  })

  it("prefers the router's reason when the reference did not resolve", () => {
    const status = credentialStatusFor(
      {
        ...anthropic,
        configured: false,
        credential_error: 'secret X not found',
      },
      { phase: 'idle' },
      0,
    )
    expect(status).toMatchObject({
      connected: false,
      error: 'secret X not found',
    })
  })
})
