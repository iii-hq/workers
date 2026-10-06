import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { DEVICE_PROVIDERS } from '@/lib/onboarding/catalog'
import { DeviceSignIn } from './DeviceSignIn'
import { ProviderSignIn } from './ProviderSignIn'

describe('DeviceSignIn', () => {
  it('starts with Authenticate and no Retry', () => {
    const html = renderToStaticMarkup(
      <DeviceSignIn provider={DEVICE_PROVIDERS[0]} />,
    )
    expect(html).toContain('Authenticate')
    expect(html).not.toContain('Retry')
  })

  it('is what the picker shows for GitHub Copilot', () => {
    const html = renderToStaticMarkup(
      <ProviderSignIn providerId="github-copilot" />,
    )
    expect(html).toContain('Sign in with GitHub')
    expect(html).toContain('Authenticate')
  })
})
