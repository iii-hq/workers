import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { DEVICE_PROVIDERS } from '@/lib/onboarding/catalog'
import { DeviceSignIn } from './DeviceSignIn'
import { ProviderSignIn } from './ProviderSignIn'

const copilot = DEVICE_PROVIDERS[0]

describe('DeviceSignIn', () => {
  it('fetches the code first when the worker runs, before any button', () => {
    const html = renderToStaticMarkup(<DeviceSignIn provider={copilot} />)
    expect(html).toContain('Getting a code')
    expect(html).not.toMatch(/>Authenticate<\/button>/)
    expect(html).not.toMatch(/Retry<\/button>/)
  })

  it('asks before adding a worker that is not running', () => {
    const html = renderToStaticMarkup(
      <DeviceSignIn provider={copilot} installed={false} />,
    )
    expect(html).toContain('Get code')
    expect(html).toContain('provider-github-copilot')
    expect(html).not.toContain('Getting a code')
  })

  it('is what the picker shows for GitHub Copilot', () => {
    const html = renderToStaticMarkup(
      <ProviderSignIn providerId="github-copilot" />,
    )
    expect(html).toContain('Sign in with GitHub')
  })
})
