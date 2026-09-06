import { describe, expect, it, vi } from 'vitest'
import {
  fetchRegistryProviders,
  parseRegistryWorkersPage,
  providerIdForRegistryWorker,
  registryWorkerMatchesProvider,
} from './workers-registry'

const ROW = {
  author: { name: 'iii', pfp: null, verified: true },
  config: {},
  dependencies: [{ name: 'llm-router', version: '^1.4.12' }],
  description: 'OpenAI Responses provider worker.',
  license: 'Apache-2.0',
  name: 'provider-openai',
  repo: 'https://github.com/iii-hq/workers',
  supported_targets: ['aarch64-apple-darwin'],
  tags: ['llm', 'openai', 'provider'],
  total_downloads: 1759,
  type: 'binary',
  version: '1.2.10',
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

describe('parseRegistryWorkersPage', () => {
  it('maps the registry row shape and pagination', () => {
    const page = parseRegistryWorkersPage({
      pagination: { has_more: true, next_cursor: 'abc', page_size: 20 },
      workers: [
        ROW,
        { name: '' },
        'junk',
        { ...ROW, name: 'cursor', tags: null, author: null },
      ],
    })
    expect(page.nextCursor).toBe('abc')
    expect(page.workers).toEqual([
      {
        name: 'provider-openai',
        description: 'OpenAI Responses provider worker.',
        version: '1.2.10',
        tags: ['llm', 'openai', 'provider'],
        totalDownloads: 1759,
        authorName: 'iii',
        authorVerified: true,
      },
      expect.objectContaining({
        name: 'cursor',
        tags: [],
        authorName: null,
        authorVerified: false,
      }),
    ])
  })

  it('treats a last page and malformed envelopes as the end', () => {
    expect(
      parseRegistryWorkersPage({
        pagination: { has_more: false, next_cursor: null },
        workers: [],
      }).nextCursor,
    ).toBeNull()
    expect(parseRegistryWorkersPage(null)).toEqual({
      workers: [],
      nextCursor: null,
    })
  })
})

describe('fetchRegistryProviders', () => {
  it('asks for the provider tag and follows the cursor', async () => {
    const fetchImpl = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        jsonResponse({
          pagination: { has_more: true, next_cursor: 'p2' },
          workers: [ROW],
        }),
      )
      .mockResolvedValueOnce(
        jsonResponse({
          pagination: { has_more: false, next_cursor: null },
          workers: [ROW, { ...ROW, name: 'provider-anthropic' }],
        }),
      )

    const workers = await fetchRegistryProviders({ fetch: fetchImpl })

    expect(workers.map((w) => w.name)).toEqual([
      'provider-openai',
      'provider-anthropic',
    ])
    const urls = fetchImpl.mock.calls.map(([input]) => String(input))
    expect(urls[0]).toBe('https://api.workers.iii.dev/w?tag=provider')
    expect(urls[1]).toBe('https://api.workers.iii.dev/w?tag=provider&cursor=p2')
  })

  it('rejects on a non-2xx status', async () => {
    const fetchImpl = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse({ error: 'down' }, 503))
    await expect(fetchRegistryProviders({ fetch: fetchImpl })).rejects.toThrow(
      'HTTP 503',
    )
  })
})

describe('provider id mapping', () => {
  it('strips the provider- prefix and compares loosely', () => {
    expect(providerIdForRegistryWorker('provider-openai')).toBe('openai')
    expect(providerIdForRegistryWorker('cursor')).toBe('cursor')
    expect(registryWorkerMatchesProvider('provider-openai', 'openai')).toBe(
      true,
    )
    expect(
      registryWorkerMatchesProvider('provider-opencode-go', 'opencode_go'),
    ).toBe(true)
    expect(
      registryWorkerMatchesProvider('provider-openai', 'openai-codex'),
    ).toBe(false)
  })
})
