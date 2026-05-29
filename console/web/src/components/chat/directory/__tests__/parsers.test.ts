import { describe, expect, it } from 'vitest'
import {
  DIRECTORY_FUNCTION_IDS,
  isDirectoryFunction,
  promptsGetResponseSchema,
  promptsListResponseSchema,
  registryWorkerInfoRequestSchema,
  registryWorkerInfoResponseSchema,
  registryWorkersListRequestSchema,
  registryWorkersListResponseSchema,
  safeParseRequest,
  safeParseResponse,
  skillsDownloadRequestSchema,
  skillsDownloadResponseSchema,
  skillsGetRequestSchema,
  skillsGetResponseSchema,
  skillsIndexResponseSchema,
  skillsListRequestSchema,
  skillsListResponseSchema,
  unwrapEnvelope,
} from '../parsers'

function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: false,
  }
}

describe('isDirectoryFunction', () => {
  it('matches every id in the allowlist', () => {
    for (const id of DIRECTORY_FUNCTION_IDS) {
      expect(isDirectoryFunction(id)).toBe(true)
    }
  })

  it('rejects unrelated ids', () => {
    expect(isDirectoryFunction('directory::')).toBe(false)
    expect(isDirectoryFunction('directory::skills')).toBe(false)
    expect(isDirectoryFunction('engine::workers::list')).toBe(false)
  })
})

describe('directory::skills::list', () => {
  it('parses an empty request', () => {
    expect(safeParseRequest(skillsListRequestSchema, {})).toEqual({})
  })

  it('parses a prefix+type filter', () => {
    expect(
      safeParseRequest(skillsListRequestSchema, {
        prefix: 'sandbox/',
        type: 'how-to',
        include_description: false,
      }),
    ).toEqual({
      prefix: 'sandbox/',
      type: 'how-to',
      include_description: false,
    })
  })

  it('parses a wrapped response payload', () => {
    const payload = {
      skills: [
        {
          id: 'sandbox/skills/sandbox/create',
          title: 'sandbox::create',
          type: 'how-to',
          function_id: 'sandbox::create',
          description: '…',
          bytes: 1840,
          modified_at: '2026-05-26T10:00:00Z',
        },
      ],
    }
    const parsed = safeParseResponse(skillsListResponseSchema, wrap(payload))
    expect(parsed?.skills[0].function_id).toBe('sandbox::create')
  })

  it('parses an entry with null type + function_id', () => {
    expect(
      safeParseResponse(skillsListResponseSchema, {
        skills: [
          {
            id: 'sandbox/index',
            title: 'sandbox',
            type: null,
            function_id: null,
            description: '',
            bytes: 100,
            modified_at: '',
          },
        ],
      }),
    ).toBeTruthy()
  })
})

describe('directory::skills::get', () => {
  it('parses the request payload', () => {
    expect(safeParseRequest(skillsGetRequestSchema, { id: 'a/b' })).toEqual({
      id: 'a/b',
    })
  })

  it('parses a wrapped get response', () => {
    const parsed = safeParseResponse(skillsGetResponseSchema, {
      id: 'a/b',
      title: 'A',
      type: 'how-to',
      function_id: null,
      body: '# A',
      modified_at: '2026-05-26T10:00:00Z',
    })
    expect(parsed?.body).toBe('# A')
  })

  it('rejects a response missing body', () => {
    expect(
      safeParseResponse(skillsGetResponseSchema, {
        id: 'x',
        title: 't',
        modified_at: '',
      }),
    ).toBeNull()
  })
})

describe('directory::skills::index', () => {
  it('parses the wrapped response', () => {
    const parsed = safeParseResponse(
      skillsIndexResponseSchema,
      wrap({ body: '## a', workers_count: 1 }),
    )
    expect(parsed?.workers_count).toBe(1)
  })
})

describe('directory::skills::download', () => {
  it('parses both source variants in the request', () => {
    expect(
      safeParseRequest(skillsDownloadRequestSchema, {
        repo: 'https://x',
        skill: 'a',
        branch: 'main',
      }),
    ).toBeTruthy()
    expect(
      safeParseRequest(skillsDownloadRequestSchema, {
        worker: 'pdfkit',
        version: '1.0.0',
      }),
    ).toBeTruthy()
  })

  it('parses the wrapped response', () => {
    const parsed = safeParseResponse(skillsDownloadResponseSchema, {
      namespace: 'sandbox',
      skills_written: ['sandbox/index'],
      prompts_written: [],
      source: { kind: 'repo' },
    })
    expect(parsed?.namespace).toBe('sandbox')
  })
})

describe('directory::prompts::list / get', () => {
  it('parses the list payload', () => {
    expect(
      safeParseResponse(promptsListResponseSchema, wrap({ prompts: [] })),
    ).toEqual({ prompts: [] })
  })

  it('parses the get payload', () => {
    const parsed = safeParseResponse(promptsGetResponseSchema, {
      name: 'a',
      description: 'b',
      body: '# a',
      modified_at: '',
    })
    expect(parsed?.body).toBe('# a')
  })
})

describe('directory::registry::workers::list', () => {
  it('parses an empty request', () => {
    expect(safeParseRequest(registryWorkersListRequestSchema, {})).toEqual({})
  })

  it('parses a search+cursor request', () => {
    expect(
      safeParseRequest(registryWorkersListRequestSchema, {
        search: 'pdf',
        cursor: 'abc',
      }),
    ).toEqual({ search: 'pdf', cursor: 'abc' })
  })

  it('parses a wrapped page with workers + pagination', () => {
    const parsed = safeParseResponse(
      registryWorkersListResponseSchema,
      wrap({
        workers: [
          {
            name: 'pdfkit',
            description: 'pdf renderer',
            type: 'binary',
            version: '1.0.0',
            total_downloads: 4823,
            author: { name: 'iii', verified: true },
          },
        ],
        pagination: {
          next_cursor: 'xyz',
          has_more: true,
          page_size: 20,
        },
      }),
    )
    expect(parsed?.workers[0].author?.verified).toBe(true)
    expect(parsed?.pagination.has_more).toBe(true)
  })
})

describe('directory::registry::workers::info', () => {
  it('parses the request shape', () => {
    expect(
      safeParseRequest(registryWorkerInfoRequestSchema, {
        name: 'pdfkit',
        tag: 'latest',
      }),
    ).toEqual({ name: 'pdfkit', tag: 'latest' })
  })

  it('parses a wrapped detail response with api reference + skills tree', () => {
    const parsed = safeParseResponse(
      registryWorkerInfoResponseSchema,
      wrap({
        worker: {
          name: 'pdfkit',
          description: 'pdf renderer',
          type: 'binary',
          version: '1.0.0',
        },
        readme: '# pdfkit',
        api_reference: {
          functions: [
            { name: 'pdfkit::render', description: 'render html→pdf' },
          ],
          triggers: [],
        },
        skills_tree: {
          skills: [{ path: 'pdfkit/index' }],
          prompts: [],
        },
      }),
    )
    expect(parsed?.api_reference.functions).toHaveLength(1)
    expect(parsed?.skills_tree.skills).toHaveLength(1)
  })

  it('rejects a response missing worker', () => {
    expect(
      safeParseResponse(registryWorkerInfoResponseSchema, {
        api_reference: { functions: [], triggers: [] },
        skills_tree: { skills: [], prompts: [] },
      }),
    ).toBeNull()
  })
})

describe('unwrapEnvelope re-export', () => {
  it('peels the harness envelope', () => {
    const inner = { skills: [] }
    expect(unwrapEnvelope(wrap(inner))).toEqual(inner)
  })
})
