import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { ProviderListEntry } from '@/lib/models-catalog'
import type { RegistryWorker } from '@/lib/workers-registry'
import { AddProviderPanel } from './AddProviderPanel'

function worker(name: string, description: string): RegistryWorker {
  return {
    name,
    description,
    version: '1.0.0',
    tags: ['provider'],
    totalDownloads: 1,
    authorName: 'iii',
    authorVerified: true,
  }
}

const REGISTRY = [
  worker('provider-openai', 'OpenAI Responses provider worker.'),
  worker('provider-anthropic', 'Anthropic Messages API provider worker.'),
  worker('provider-github-copilot', 'GitHub Copilot provider worker.'),
  worker('cursor', 'Cursor CLI provider worker.'),
]

const ANTHROPIC: ProviderListEntry = {
  id: 'anthropic',
  display_name: 'Anthropic',
  supports_model_listing: true,
  configured: true,
  available: true,
}

describe('AddProviderPanel', () => {
  it('lists only the registry providers that are not installed yet', () => {
    const html = renderToStaticMarkup(
      <AddProviderPanel
        registryWorkers={REGISTRY}
        installedWorkerNames={['provider-openai']}
        providers={[ANTHROPIC]}
      />,
    )
    // Installed through the engine list, and present in the router: both gone.
    expect(html).not.toContain('OpenAI Responses provider worker.')
    expect(html).not.toContain('Anthropic Messages API provider worker.')
    expect(html).toContain('GitHub Copilot provider worker.')
    expect(html).toContain('Cursor CLI provider worker.')
    expect(html.match(/aria-label="Add [^"]+"/g)).toHaveLength(2)
  })

  it('says so when every registry provider is already installed', () => {
    const html = renderToStaticMarkup(
      <AddProviderPanel
        registryWorkers={REGISTRY}
        installedWorkerNames={REGISTRY.map((entry) => entry.name)}
        providers={[]}
      />,
    )
    expect(html).toContain('already installed')
    expect(html).not.toContain('aria-label="Add ')
  })

  it('shows the registry-empty copy when the registry has no providers', () => {
    const html = renderToStaticMarkup(
      <AddProviderPanel registryWorkers={[]} installedWorkerNames={[]} />,
    )
    expect(html).toContain('lists no provider workers')
  })
})
