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
  entryRefs,
  fieldErrors,
  hideLiterals,
  literalSecrets,
  literalSecretsError,
  mapAsJson,
  READ_MAP_ROOT,
  REFERENCE,
  readEntryEdit,
  readMapEdit,
  referenceOnly,
  renameRow,
  sameJson,
  secretVaultName,
} from '../src/config-model'
import spec from './fixtures/spec-config.json'

const SPEC = spec as Record<string, Record<string, unknown>>
const rt = (name: string, entry: unknown) => entryFromDraft(draftFromEntry(name, entry))
const servers = (text: string) => (JSON.parse(text) as { mcpServers: Record<string, unknown> }).mcpServers
// A stored value that no output of the page may hold.
const LITERAL = 'abc999'
const HIDDEN = '<hidden: store it as a secret>'

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
      keyed: { type: 'http', url: 'https://x/mcp', apiKey: 'secret://K' },
      leaky: { type: 'http', url: 'https://x/mcp', apiKey: `sk-${LITERAL}` },
    })
    expect(text).not.toMatch(/secret:\/\/|env:\/\//)
    for (const value of ['abc123', 'abc456', 'abc789', LITERAL]) expect(text).not.toContain(value)
    const copied = servers(text) as Record<string, Record<string, Record<string, unknown>>>
    expect(copied.bad.headers.Authorization).toEqual('${BAD_AUTHORIZATION}')
    expect(copied.worse.headers.Authorization).toEqual('Bearer ${WORSE_TOKEN}')
    expect(copied.env.env.GITHUB_TOKEN).toEqual('${ENV_GITHUB_TOKEN}')
    expect(copied.keyed.apiKey).toEqual('${K}')
    expect(copied.leaky.apiKey).toEqual('${LEAKY_APIKEY}')
  })

  it('writes a field the form does not show as other clients read it; args stay as written', () => {
    const text = copyAsJson({
      a: {
        type: 'stdio',
        command: 'x',
        args: ['--key', LITERAL],
        apiKey: `sk-${LITERAL}`,
        region: 'env://REGION',
        note: 'plain',
        timeout: 30,
      },
    })
    expect(servers(text).a).toEqual({
      type: 'stdio',
      command: 'x',
      args: ['--key', LITERAL],
      apiKey: '${A_APIKEY}',
      region: '${REGION}',
      note: 'plain',
      timeout: 30,
    })
  })
})

describe('secret positions', () => {
  const entry = {
    type: 'http',
    url: 'https://x/mcp',
    headers: { Authorization: `Bearer ${LITERAL}`, 'X-Workspace': LITERAL, 'X-API-Key': 'secret://K', Cookie: '' },
    env: { GITHUB_TOKEN: LITERAL, HOME: '/home/me' },
    auth: { type: 'client_credentials', client_id: LITERAL, client_secret: LITERAL, token: 'env://T' },
    apiKey: `sk-${LITERAL}`,
    note: LITERAL,
    timeout: 30,
  }

  it('literalSecrets names every value in a secret position, never the value', () => {
    const paths = literalSecrets(entry)
    expect(paths).toEqual(['headers/Authorization', 'env/GITHUB_TOKEN', 'auth/client_secret', 'apiKey'])
    expect(paths.join(' ')).not.toContain(LITERAL)
    expect(literalSecrets({ auth: { type: 'bearer', token: LITERAL } })).toEqual(['auth/token'])
    expect(literalSecrets({ auth: { type: 'oauth', client_secret: LITERAL } })).toEqual(['auth/client_secret'])
    expect(literalSecrets({ auth: LITERAL })).toEqual(['auth'])
    for (const name of Object.keys(SPEC)) expect(literalSecrets(SPEC[name])).toEqual([])
    for (const bad of [null, 42, LITERAL, [LITERAL], { headers: LITERAL, env: [LITERAL] }]) {
      expect(literalSecrets(bad)).toEqual([])
    }
  })

  it('literalSecretsError refuses an entry with a value and names its paths', () => {
    expect(literalSecretsError({ auth: { type: 'bearer', token: LITERAL } })).toEqual(
      'auth/token holds a value. Use secret://NAME or env://NAME.',
    )
    const error = literalSecretsError(entry)
    expect(error).toEqual(
      'headers/Authorization, env/GITHUB_TOKEN, auth/client_secret, apiKey hold a value. Use secret://NAME or env://NAME.',
    )
    expect(literalSecretsError(SPEC.search)).toBeNull()
  })

  it('renameRow clears a value when the new name looks secret', () => {
    expect(renameRow({ name: 'X-Trace', value: LITERAL }, 'Authorization')).toEqual({
      name: 'Authorization',
      value: '',
    })
    expect(renameRow({ name: 'X', value: 'secret://K' }, 'X-Token')).toEqual({ name: 'X-Token', value: 'secret://K' })
    expect(renameRow({ name: 'X', value: '' }, 'Cookie')).toEqual({ name: 'Cookie', value: '' })
    expect(renameRow({ name: 'X', value: LITERAL }, 'X-Trace')).toEqual({ name: 'X-Trace', value: LITERAL })
  })

  it('draftFromEntry reads a value in a known secret position as empty', () => {
    const draft = draftFromEntry('a', entry)
    expect(draft.headers).toEqual([
      { name: 'Authorization', value: '' },
      { name: 'X-Workspace', value: LITERAL },
      { name: 'X-API-Key', value: 'secret://K' },
      { name: 'Cookie', value: '' },
    ])
    expect(draft.env).toEqual([
      { name: 'GITHUB_TOKEN', value: '' },
      { name: 'HOME', value: '/home/me' },
    ])
    expect(draft.clientCredentials).toMatchObject({ client_id: LITERAL, client_secret: '' })
    const bearer = { type: 'http', url: 'https://x/mcp', auth: { type: 'bearer', token: LITERAL } }
    expect(draftFromEntry('a', bearer).bearer.token).toEqual('')
    const oauth = { type: 'http', url: 'https://x/mcp', auth: { type: 'oauth', client_secret: LITERAL } }
    expect(draftFromEntry('a', oauth).oauth.client_secret).toEqual('')
    expect(draftFromEntry('a', SPEC.linear).bearer.token).toEqual('secret://LINEAR_API_KEY')
  })

  it('referenceOnly lets a reference through and nothing else', () => {
    expect(referenceOnly('secret://K')).toEqual('secret://K')
    expect(referenceOnly('env://K')).toEqual('env://K')
    expect(referenceOnly('')).toEqual('')
    expect(referenceOnly(LITERAL)).toEqual('')
    expect(referenceOnly(`secret://K ${LITERAL}`)).toEqual('')
  })

  it('keeps a secret-named field the form does not show, and hides its value when shown', () => {
    const draft = draftFromEntry('a', { type: 'http', url: 'https://x/mcp', apiKey: `sk-${LITERAL}`, timeout: 30 })
    expect(entryFromDraft(draft).apiKey).toEqual(`sk-${LITERAL}`)
    const shown = hideLiterals(entryFromDraft(draft))
    expect(shown).toEqual({ type: 'http', url: 'https://x/mcp', apiKey: HIDDEN, timeout: 30 })
    expect(JSON.stringify(hideLiterals(entry))).not.toMatch(/Bearer|sk-/)
    expect(hideLiterals(entry)).toMatchObject({ note: LITERAL, headers: { 'X-API-Key': 'secret://K', Cookie: '' } })
    expect(literalSecrets(shown)).toEqual(['apiKey'])
    expect(hideLiterals(SPEC.linear)).toEqual(SPEC.linear)
  })

  it('a secret-named key inside an auth of a known type is a secret position, token_url is not', () => {
    const auth = {
      type: 'client_credentials',
      token_url: 'https://x/oauth/token',
      client_id: 'id',
      client_secret: 'secret://S',
      scope: 'read',
    }
    const stored = { s: { type: 'http', url: 'https://x/mcp', auth } }
    const entry = { ...stored.s, auth: { ...auth, refresh_token: LITERAL } }
    expect(hideLiterals(entry)).toEqual({ ...entry, auth: { ...auth, refresh_token: HIDDEN } })
    expect(literalSecretsError(entry)).toEqual('auth/refresh_token holds a value. Use secret://NAME or env://NAME.')
    expect(literalSecretsError(stored.s)).toBeNull()
    expect(readMapEdit(stored, JSON.stringify({ mcpServers: { s: entry } }))).toEqual({
      issues: [
        { path: '/mcpServers/s', message: 'auth/refresh_token holds a value. Use secret://NAME or env://NAME.' },
      ],
    })
    // An auth of unknown type is not a secret position past `token` and `client_secret`.
    expect(literalSecrets({ auth: { type: 'custom', refresh_token: LITERAL } })).toEqual([])
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

describe('entryRefs', () => {
  it('lists every reference of an entry once, sorted, and no literal', () => {
    expect(
      entryRefs({
        url: 'https://x/mcp',
        auth: { type: 'bearer', token: 'secret://B_KEY' },
        headers: { 'X-API-Key': 'env://A_KEY', 'X-Workspace': 'acme' },
        args: ['secret://B_KEY', '--flag'],
      }),
    ).toEqual(['env://A_KEY', 'secret://B_KEY'])
    expect(entryRefs(null)).toEqual([])
  })
})

describe('the JSON views read an edit against the text they showed', () => {
  // A stored value the page hides: an unknown secret-named key, and a bearer token.
  const stored = {
    a: { type: 'http', url: 'https://a.example/mcp', api_key: LITERAL },
    b: { type: 'http', url: 'https://b.example/mcp', auth: { type: 'bearer', token: LITERAL } },
    linear: SPEC.linear,
  }

  it('readEntryEdit reads the shown text, hidden values and all, as no edit', () => {
    const draft = draftFromEntry('a', stored.a)
    const shown = JSON.stringify(hideLiterals(entryFromDraft(draft)), null, 2)
    expect(shown).toContain(HIDDEN)
    expect(readEntryEdit(draft, shown)).toEqual({ draft: null, error: null })
    const edited = { ...hideLiterals(entryFromDraft(draft)), url: 'https://c.example/mcp' }
    expect(readEntryEdit(draft, JSON.stringify(edited))).toEqual({
      draft: null,
      error: 'api_key holds a value. Use secret://NAME or env://NAME.',
    })
    const fixed = { ...edited, api_key: 'secret://A_KEY' }
    expect(readEntryEdit(draft, JSON.stringify(fixed))).toEqual({ draft: draftFromEntry('a', fixed), error: null })
    expect(readEntryEdit(draft, '{').error).toMatch(/^Not valid JSON/)
  })

  it('readMapEdit passes untouched hidden entries through and refuses a value in a changed one', () => {
    const shown = mapAsJson(stored)
    expect(shown).not.toContain(LITERAL)
    expect(readMapEdit(stored, shown)).toEqual({ candidate: stored, changed: [], removed: [] })
    const servers = (JSON.parse(shown) as { mcpServers: Record<string, Record<string, unknown>> }).mcpServers
    const linear = { ...SPEC.linear, url: 'https://linear.example/mcp' }
    const { a: _, ...edited } = { ...servers, linear }
    expect(readMapEdit(stored, JSON.stringify({ mcpServers: edited }))).toEqual({
      candidate: { b: stored.b, linear },
      changed: ['linear'],
      removed: ['a'],
    })
    const kept = { ...servers, b: { ...servers.b, url: 'https://c.example/mcp' } }
    expect(readMapEdit(stored, JSON.stringify({ mcpServers: kept }))).toEqual({
      issues: [{ path: '/mcpServers/b', message: 'auth/token holds a value. Use secret://NAME or env://NAME.' }],
    })
    expect(readMapEdit(stored, '[]')).toEqual({ issues: [{ path: '', message: READ_MAP_ROOT }] })
    expect(readMapEdit(stored, '{').issues?.[0].message).toMatch(/^Not valid JSON/)
  })
})
