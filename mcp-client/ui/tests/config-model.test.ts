// biome-ignore-all lint/suspicious/noTemplateCurlyInString: the portable copy spells a reference `${NAME}`
import { describe, expect, it } from 'vitest'
import {
  changedEntries,
  connectionChanged,
  copyAsJson,
  draftDirty,
  draftFromEntry,
  emptyDraft,
  entryFromDraft,
  fieldErrors,
  REFERENCE,
  sameJson,
  secretVaultName,
} from '../src/config-model'
import spec from './fixtures/spec-config.json'

const SPEC = spec as Record<string, Record<string, unknown>>
const rt = (name: string, entry: unknown) => entryFromDraft(draftFromEntry(name, entry))
const servers = (text: string) => (JSON.parse(text) as { mcpServers: Record<string, unknown> }).mcpServers

describe('draftFromEntry and entryFromDraft', () => {
  it.each(Object.keys(SPEC))('%s survives the round trip', (name) => {
    const { disabled: _, ...expected } = SPEC[name]
    expect(rt(name, SPEC[name])).toEqual(expected)
  })

  it('infers the transport and always writes type', () => {
    expect(rt('a', { command: 'npx', args: ['x'] })).toEqual({ type: 'stdio', command: 'npx', args: ['x'] })
    expect(rt('a', { type: 'streamable-http', url: 'https://x/mcp' })).toEqual({ type: 'http', url: 'https://x/mcp' })
  })

  it('keeps unknown fields, and an auth the form cannot show', () => {
    const entry = {
      type: 'http',
      url: 'https://x/mcp',
      alwaysLoad: true,
      timeout: 30,
      auth: { type: 'basic', user: 'u' },
    }
    const draft = draftFromEntry('a', entry)
    expect(draft.extra).toEqual({ alwaysLoad: true, timeout: 30, auth: { type: 'basic', user: 'u' } })
    expect(draft.authMode).toEqual('none')
    expect(entryFromDraft(draft)).toEqual(entry)
    expect(entryFromDraft({ ...draft, authMode: 'oauth' }).auth).toEqual({ type: 'oauth' })
  })

  it('writes only the chosen mode and transport, and keeps the rest in the draft', () => {
    const draft = {
      ...draftFromEntry('linear', SPEC.linear),
      authMode: 'client_credentials' as const,
      clientCredentials: {
        token_url: 'https://api.linear.app/oauth/token',
        client_id: 'abc123',
        client_secret: 'secret://LINEAR_CLIENT_SECRET',
        scope: '',
      },
    }
    expect(entryFromDraft(draft).auth).toEqual({
      type: 'client_credentials',
      token_url: 'https://api.linear.app/oauth/token',
      client_id: 'abc123',
      client_secret: 'secret://LINEAR_CLIENT_SECRET',
    })
    expect(draft.bearer.token).toEqual('secret://LINEAR_API_KEY')
    expect(entryFromDraft({ ...draft, transport: 'stdio', command: 'npx' })).toEqual({ type: 'stdio', command: 'npx' })
  })

  it('drops empty rows and empty optional values', () => {
    expect(
      entryFromDraft({
        ...emptyDraft(),
        name: 'a',
        url: 'https://x/mcp',
        disabled: true,
        authMode: 'oauth',
        headers: [
          { name: '', value: '' },
          { name: 'X-Workspace', value: 'acme' },
        ],
        oauth: { scopes: 'read, write', client_id: '', client_secret: '', client_metadata_url: '' },
      }),
    ).toEqual({
      type: 'http',
      url: 'https://x/mcp',
      headers: { 'X-Workspace': 'acme' },
      disabled: true,
      auth: { type: 'oauth', scopes: ['read', 'write'] },
    })
    expect(emptyDraft()).toMatchObject({
      name: '',
      transport: 'http',
      authMode: 'none',
      disabled: false,
      headers: [],
      args: [],
      env: [],
      extra: {},
    })
  })

  it('never throws on a malformed entry', () => {
    for (const bad of [null, 42, 'x', [], { headers: 7, args: 'x', env: null, auth: 'x' }]) {
      expect(() => rt('a', bad)).not.toThrow()
    }
  })
})

describe('copyAsJson', () => {
  it('writes the form other clients read', () => {
    const text = copyAsJson(SPEC)
    expect(text.startsWith('{\n  "mcpServers"')).toBe(true)
    const linearOnly = { type: 'http', url: 'https://mcp.linear.app/mcp' }
    expect(servers(text)).toEqual({
      linear: {
        type: 'http',
        url: 'https://mcp.linear.app/mcp',
        headers: { Authorization: 'Bearer ${LINEAR_API_KEY}' },
      },
      'linear-app': linearOnly,
      'linear-me': linearOnly,
      'linear-auto': linearOnly,
      search: {
        type: 'http',
        url: 'https://mcp.example.com/mcp',
        headers: { 'X-API-Key': '${SEARCH_X_API_KEY}', 'X-Workspace': 'acme' },
      },
      playwright: {
        type: 'stdio',
        command: 'npx',
        args: ['-y', '@playwright/mcp@0.0.41'],
        cwd: '/home/me/project',
        env: {
          PLAYWRIGHT_BROWSERS_PATH: '/opt/pw-browsers',
          GITHUB_TOKEN: '${PLAYWRIGHT_GITHUB_TOKEN}',
          HTTPS_PROXY: '${HTTPS_PROXY}',
        },
      },
    })
  })

  it('maps a pre-registered OAuth client and infers type', () => {
    const text = copyAsJson({
      a: {
        url: 'https://x/mcp',
        auth: { type: 'oauth', client_id: 'cid', client_secret: 'secret://S', scopes: ['read'] },
      },
    })
    expect(servers(text).a).toEqual({ type: 'http', url: 'https://x/mcp', oauth: { clientId: 'cid' } })
  })

  it('never contains a reference scheme or a stored value', () => {
    const text = copyAsJson({
      ...SPEC,
      bad: { type: 'http', url: 'https://x/mcp', headers: { Authorization: 'Bearer abc123' } },
      worse: { type: 'http', url: 'https://x/mcp', auth: { type: 'bearer', token: 'abc456' } },
      env: { type: 'stdio', command: 'x', env: { GITHUB_TOKEN: 'abc789' } },
    })
    expect(text).not.toMatch(/secret:\/\/|env:\/\//)
    for (const value of ['abc123', 'abc456', 'abc789']) expect(text).not.toContain(value)
    const copied = servers(text) as Record<string, Record<string, Record<string, unknown>>>
    expect(copied.bad.headers.Authorization).toEqual('${BAD_AUTHORIZATION}')
    expect(copied.worse.headers.Authorization).toEqual('Bearer ${WORSE_TOKEN}')
    expect(copied.env.env.GITHUB_TOKEN).toEqual('${ENV_GITHUB_TOKEN}')
  })
})

describe('helpers', () => {
  it('sameJson ignores key order and nothing else', () => {
    expect(sameJson({ a: 1, b: [1, { c: 2, d: 3 }] }, { b: [1, { d: 3, c: 2 }], a: 1 })).toEqual(true)
    expect(sameJson(['-y', 'x'], ['x', '-y'])).toEqual(false)
    expect(sameJson({ a: undefined }, {})).toEqual(true)
  })

  it('connectionChanged looks at url, command and args only', () => {
    const http = SPEC.search
    expect(connectionChanged(http, { ...http, headers: {} })).toEqual(false)
    expect(connectionChanged(http, { ...http, url: 'https://other/mcp' })).toEqual(true)
    expect(connectionChanged(null, http)).toEqual(true)
    const stdio = SPEC.playwright
    expect(connectionChanged(stdio, { ...stdio, env: {}, cwd: '/tmp' })).toEqual(false)
    expect(connectionChanged(stdio, { ...stdio, command: 'bunx' })).toEqual(true)
    expect(connectionChanged(stdio, { ...stdio, args: ['@playwright/mcp@0.0.41', '-y'] })).toEqual(true)
  })

  it('changedEntries names what a JSON edit touched', () => {
    expect(
      changedEntries(
        { a: { url: 'u', type: 'http' }, b: { command: 'x' }, c: {} },
        { a: { type: 'http', url: 'u' }, b: { command: 'y' }, d: {} },
      ),
    ).toEqual({ changed: ['b', 'd'], removed: ['c'] })
  })

  it('secretVaultName builds a valid secret name', () => {
    const cases: [string, string, string][] = [
      ['linear', 'TOKEN', 'LINEAR_TOKEN'],
      ['search', 'X-API-Key', 'SEARCH_X_API_KEY'],
      ['linear-app', 'client_secret', 'LINEAR_APP_CLIENT_SECRET'],
      ['9x', 'TOKEN', '_9X_TOKEN'],
      ['a', 'x~y!z', 'A_X_Y_Z'],
    ]
    for (const [server, field, expected] of cases) {
      expect(secretVaultName(server, field)).toEqual(expected)
      expect(secretVaultName(server, field)).toMatch(/^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/)
    }
  })

  it('fieldErrors keys messages by the field they belong to', () => {
    expect(
      fieldErrors(
        [
          { path: '/mcpServers/search/headers/X-API-Key', message: 'm1' },
          { path: '/mcpServers/search/auth/token', message: 'm2' },
          { path: '/mcpServers/search', message: 'm3' },
          { path: '/mcpServers/other/url', message: 'no' },
        ],
        'search',
      ),
    ).toEqual({ 'headers/X-API-Key': 'm1', 'auth/token': 'm2', '': 'm3' })
  })

  it('draftDirty compares with the stored entry, or with an empty draft while adding', () => {
    expect(draftDirty(draftFromEntry('playwright', SPEC.playwright), SPEC.playwright)).toEqual(false)
    expect(draftDirty({ ...draftFromEntry('linear', SPEC.linear), url: 'https://x/mcp' }, SPEC.linear)).toEqual(true)
    expect(draftDirty(emptyDraft(), undefined)).toEqual(false)
    expect(draftDirty({ ...emptyDraft(), name: 'a' }, undefined)).toEqual(true)
  })

  it('REFERENCE accepts a whole reference only', () => {
    for (const text of ['secret://LINEAR_API_KEY', 'env://HTTPS_PROXY']) expect(REFERENCE.test(text)).toEqual(true)
    for (const text of ['', 'abc', 'Bearer secret://X', 'secret://', 'secret://A B', '${X}']) {
      expect(REFERENCE.test(text)).toEqual(false)
    }
  })
})
