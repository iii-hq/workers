// biome-ignore-all lint/suspicious/noTemplateCurlyInString: pasted configs spell references `${VAR}`
import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import {
  crossRowBlockers,
  finalEntry,
  type ImportDeps,
  importEffects,
  type ImportResult,
  type ImportRow,
  importRow,
  importSummary,
  maskSecret,
  mergeConsumers,
  parseImport,
  renameRow,
  rowBlockers,
  scrub,
  slugName,
  startValues,
} from '../src/importer'

// Fixtures hold the marker `@@FAKE@@` where a pasted secret goes, so no file holds a credential shape.
const FAKE = 'test-not-a-real-token'
const MASK = '•••••••• · 21 characters'
const COMMON = 'Claude Code, Cursor and similar'
const FIXTURES = new URL('./fixtures/import/', import.meta.url)
const load = (name: string) => readFileSync(new URL(name, FIXTURES), 'utf8').replaceAll('@@FAKE@@', FAKE)
const parse = (name: string, existing: string[] = []) => parseImport(load(name), existing)
function row(result: ImportResult, name: string): ImportRow {
  const found = result.rows.find((r) => r.name === name)
  if (!found) throw new Error(`no row ${JSON.stringify(name)}; rows: ${JSON.stringify(result.rows.map((r) => r.name))}`)
  return found
}
const fields = (r: ImportRow) => r.ignored.map(({ field, note }) => (note ? `${field} (${note})` : field)).sort()
const skipped = (file: string, name: string) => {
  const r = row(parse(file), name)
  return [r.status, r.transport, r.skipReason]
}
// Every fixture file except codex.toml and native.json.
const CLIENTS = readdirSync(FIXTURES).filter((file) => file !== 'codex.toml' && file !== 'native.json')

describe('detection', () => {
  it.each<[string, string, string, string[]]>([
    ['claude-code.json', 'mcpServers', COMMON, ['api-server', 'weather', 'slack-team', 'asana', 'linear-server']],
    ['claude-desktop.json', 'mcpServers', COMMON, ['filesystem', 'github']],
    ['cursor.json', 'mcpServers', COMMON, ['local-server', 'remote-server', 'oauth-server', 'keyed']],
    ['windsurf.json', 'mcpServers', COMMON, ['github', 'remote-http-mcp']],
    ['gemini.json', 'mcpServers', 'Gemini CLI', ['local', 'streaming', 'legacy']],
    ['vscode-editor.json', 'vscode-editor', 'VS Code', ['github', 'perplexity', 'oauthed']],
    ['vscode-portable.json', 'mcpServers', COMMON, ['playwright', 'context7']],
    ['zed.json', 'zed', 'Zed', ['local-mcp-server', 'remote-mcp-server', 'remote-mcp-server-with-o']],
    ['acp.json', 'acp', 'ACP', ['filesystem', 'api-server', 'events']],
    ['registry.json', 'registry', 'MCP registry', ['weather', 'weather-2']],
    ['single.json', 'single', 'One server', ['']],
    ['bare-map.json', 'bare-map', 'Server map', ['docs', 'tools']],
    ['native.json', 'mcpServers', COMMON, ['linear', 'linear-app', 'search', 'playwright']],
  ])('%s is %s', (file, source, label, names) => {
    const result = parse(file)
    expect([result.source, result.sourceLabel, result.error]).toEqual([source, label, undefined])
    expect(result.rows.map((r) => r.name)).toEqual(names)
  })
  it('answers Codex TOML and bad input without echoing the input', () => {
    expect(parse('codex.toml')).toEqual({
      source: 'codex-toml',
      sourceLabel: 'Codex TOML',
      rows: [],
      error: 'Paste JSON, or add the server by hand',
    })
    expect(parseImport(`{"mcpServers": {"a": {"env": {"API_KEY": "${FAKE}"}}`, [])).toEqual({
      source: 'unknown',
      sourceLabel: 'Not recognised',
      rows: [],
      error: 'Not valid JSON',
    })
    expect(parseImport('{"foo": 1}', []).error).toBe('No MCP servers found in this JSON')
    expect(parseImport('{"name": "x/y", "packages": [{}]}', []).error).toBe(
      'This registry entry has no remote server. Add the server by hand',
    )
  })
  it('skips what mcp-client cannot connect to, with the reason', () => {
    // skipped(file, name) is [status, transport, skipReason] of that row
    expect(skipped('claude-code.json', 'asana')).toEqual(['skipped', null, 'SSE is not supported'])
    expect(skipped('gemini.json', 'legacy')).toEqual([
      'skipped',
      null,
      'Gemini\'s "url" is an SSE endpoint, which is not supported',
    ])
    expect(skipped('acp.json', 'events')).toEqual(['skipped', null, 'SSE is not supported'])
    expect(skipped('registry.json', 'weather-2')).toEqual(['skipped', null, 'SSE is not supported'])
    const odd = parseImport(
      JSON.stringify({
        mcpServers: { a: { type: 'ws', url: 'wss://x.example.com' }, b: { type: 'sdk' }, c: {}, d: 7 },
      }),
      [],
    )
    expect(odd.rows.map((r) => r.skipReason)).toEqual([
      'WebSocket is not supported',
      'In-process SDK servers cannot be imported',
      'No command and no url',
      'Not a server entry',
    ])
  })
  it('marks existing names, slugs the rest and keeps slugs apart', () => {
    expect(parse('claude-code.json', ['linear-server']).rows.map((r) => [r.originalName, r.name, r.status])).toEqual([
      ['api-server', 'api-server', 'new'],
      ['weather', 'weather', 'new'],
      ['Slack_Team', 'slack-team', 'new'],
      ['asana', 'asana', 'skipped'],
      ['linear-server', 'linear-server', 'replaces'],
    ])
    const twice = parseImport(JSON.stringify({ A_b: { command: 'x' }, 'a-b': { command: 'y' } }), [])
    expect(twice.rows.map((r) => r.name)).toEqual(['a-b', 'a-b-2'])
  })
  it.each([
    ['linear-server', 'linear-server'],
    ['My_Server.v2', 'my-server-v2'],
    ['--X--', 'x'],
    ['remote-mcp-server-with-oauth', 'remote-mcp-server-with-o'],
    ['日本', 'server'],
    ['a'.repeat(30), 'a'.repeat(24)],
  ])('slugName(%s)', (input, out) => {
    expect(slugName(input)).toBe(out)
    expect(out).toMatch(/^[a-z0-9][a-z0-9-]{0,23}$/)
  })
  it('lists ignored fields and never reads a helper', () => {
    const helpers = parse('headers-helper.json')
    expect(fields(row(helpers, 'internal-api'))).toEqual(['headersHelper (not run)'])
    expect(fields(row(helpers, 'codex-style'))).toEqual(['http_headers_helper (not run)'])
    expect(JSON.stringify(helpers)).not.toMatch(/get-mcp-auth-headers|print-headers/)
    expect(fields(row(parse('claude-code.json'), 'slack-team'))).toEqual([
      'oauth.authServerMetadataUrl',
      "oauth.callbackPort (the other client's callback)",
    ])
    expect(fields(row(parse('cursor.json'), 'local-server'))).toEqual(['envFile (not read)'])
    expect(fields(row(parse('gemini.json'), 'local'))).toEqual(['timeout', 'trust'])
    expect(fields(row(parse('gemini.json'), 'streaming'))).toEqual([
      'includeTools',
      "oauth.redirectUri (the other client's callback)",
      'oauth.tokenUrl',
    ])
    expect(fields(row(parse('vscode-editor.json'), 'perplexity'))).toEqual([
      'dev',
      'envFile (not read)',
      'inputs.build-id (not run)',
    ])
    expect(fields(row(parse('acp.json'), 'api-server'))).toEqual(['headers.Content-Type (set by mcp-client)'])
    expect(fields(row(parse('bare-map.json'), 'docs'))).toEqual(['enabled'])
    expect(fields(row(parse('bare-map.json'), 'tools'))).toEqual([])
  })
})

describe('entries', () => {
  it.each<[string, string, Record<string, unknown>]>([
    ['claude-code.json', 'api-server', { type: 'http', url: '', auth: { type: 'bearer', token: 'env://API_KEY' } }],
    [
      'claude-code.json',
      'weather',
      {
        type: 'stdio',
        command: '/opt/weather/weather-cli',
        args: ['--api-key', '', '--region', 'env://REGION'],
        env: { CACHE_DIR: '/tmp/weather', LOG_LEVEL: 'env://LOG_LEVEL' },
      },
    ],
    [
      'claude-code.json',
      'slack-team',
      {
        type: 'http',
        url: 'https://mcp.slack.com/mcp',
        auth: { type: 'oauth', client_id: 'slack-client', scopes: ['channels:read', 'chat:write'] },
      },
    ],
    ['claude-code.json', 'linear-server', { type: 'http', url: 'https://mcp.linear.app/mcp' }],
    [
      'claude-desktop.json',
      'github',
      {
        type: 'stdio',
        command: 'npx',
        args: ['-y', '@modelcontextprotocol/server-github'],
        env: { GITHUB_PERSONAL_ACCESS_TOKEN: '' },
      },
    ],
    [
      'cursor.json',
      'local-server',
      { type: 'stdio', command: 'python', args: [''], env: { API_KEY: 'env://API_KEY' } },
    ],
    [
      'cursor.json',
      'remote-server',
      { type: 'http', url: 'https://api.example.com/mcp', auth: { type: 'bearer', token: 'env://MY_SERVICE_TOKEN' } },
    ],
    [
      'cursor.json',
      'oauth-server',
      {
        type: 'http',
        url: 'https://api.example.com/mcp',
        auth: { type: 'oauth', client_id: 'cursor-client', client_secret: '', scopes: ['read', 'write'] },
      },
    ],
    [
      'cursor.json',
      'keyed',
      { type: 'http', url: 'https://api.example.com/mcp', headers: { API_KEY: '', 'X-Workspace': 'acme' } },
    ],
    [
      'windsurf.json',
      'remote-http-mcp',
      { type: 'http', url: 'https://mcp.example.com/mcp', headers: { API_KEY: '' } },
    ],
    [
      'gemini.json',
      'local',
      {
        type: 'stdio',
        command: 'path/to/server',
        args: ['--arg1', 'value1'],
        env: { API_KEY: 'env://MY_API_TOKEN', WIN_KEY: 'env://MY_WIN_TOKEN' },
        cwd: '',
      },
    ],
    [
      'gemini.json',
      'streaming',
      {
        type: 'http',
        url: 'https://api.example.com/mcp',
        headers: { 'X-Workspace': 'acme' },
        auth: { type: 'oauth', client_id: 'gemini-client', client_secret: '', scopes: ['read'] },
      },
    ],
    [
      'vscode-editor.json',
      'github',
      { type: 'http', url: 'https://api.githubcopilot.com/mcp', auth: { type: 'bearer', token: '' } },
    ],
    [
      'vscode-editor.json',
      'perplexity',
      {
        type: 'stdio',
        command: 'npx',
        args: ['-y', 'server-perplexity-ask'],
        env: { PERPLEXITY_API_KEY: '', REGION: '', BUILD: '', RETRIES: '3' },
        cwd: '',
      },
    ],
    [
      'vscode-portable.json',
      'context7',
      {
        type: 'http',
        url: 'https://mcp.context7.com/mcp',
        headers: { 'X-Api-Version': 'env://API_VERSION' },
        auth: { type: 'oauth', client_id: 'portable-client' },
      },
    ],
    ['zed.json', 'local-mcp-server', { type: 'stdio', command: 'some-command', args: ['arg-1', 'arg-2'] }],
    [
      'zed.json',
      'remote-mcp-server',
      { type: 'http', url: 'https://example.com/mcp', auth: { type: 'bearer', token: '' } },
    ],
    [
      'acp.json',
      'filesystem',
      { type: 'stdio', command: '/path/to/mcp-server', args: ['--stdio'], env: { API_KEY: '' } },
    ],
    [
      'acp.json',
      'api-server',
      { type: 'http', url: 'https://api.example.com/mcp', auth: { type: 'bearer', token: '' } },
    ],
    [
      'registry.json',
      'weather',
      {
        type: 'http',
        url: 'https://mcp.anonymous.modelcontextprotocol.io/http',
        headers: { 'X-API-Key': '', 'X-Region': '' },
      },
    ],
    ['single.json', '', { type: 'http', url: 'https://api.weather.com/mcp', auth: { type: 'bearer', token: '' } }],
    [
      'bare-map.json',
      'docs',
      {
        type: 'http',
        url: 'https://docs.example.com/mcp',
        headers: { 'X-Workspace': 'acme', 'X-Tenant': 'env://TENANT_ID' },
        auth: { type: 'bearer', token: 'env://DOCS_TOKEN' },
      },
    ],
    [
      'bare-map.json',
      'tools',
      { type: 'stdio', command: 'uvx', args: ['some-tools'], env: { LOCAL_TOKEN: 'env://LOCAL_TOKEN' } },
    ],
    ['headers-helper.json', 'internal-api', { type: 'http', url: 'https://mcp.example.org' }],
  ])('%s › %s', (file, name, entry) => expect(row(parse(file), name).entry).toEqual(entry))
  it.each<[string, string, [string, string][]]>([
    ['claude-code.json', 'weather', [['/args/1', 'WEATHER_API_KEY']]],
    ['claude-desktop.json', 'github', [['/env/GITHUB_PERSONAL_ACCESS_TOKEN', 'GITHUB_GITHUB_PERSONAL_ACCESS_TOKEN']]],
    ['cursor.json', 'oauth-server', [['/auth/client_secret', 'OAUTH_SERVER_CLIENT_SECRET']]],
    ['cursor.json', 'keyed', [['/headers/API_KEY', 'KEYED_API_KEY']]],
    ['gemini.json', 'streaming', [['/auth/client_secret', 'STREAMING_CLIENT_SECRET']]],
    ['acp.json', 'filesystem', [['/env/API_KEY', 'FILESYSTEM_API_KEY']]],
    ['acp.json', 'api-server', [['/auth/token', 'API_SERVER_TOKEN']]],
    ['native.json', 'linear', [['/auth/token', 'LINEAR_TOKEN']]],
    ['native.json', 'search', [['/headers/X-API-Key', 'SEARCH_X_API_KEY']]],
  ])('%s › %s secrets', (file, name, found) => {
    const secrets = row(parse(file), name).secrets
    expect(secrets.map((s) => [s.path, s.vaultName])).toEqual(found)
    for (const s of secrets) expect([s.value, s.masked]).toEqual([FAKE, MASK])
  })
  it.each<[string, string, [string, string, boolean, string][]]>([
    ['claude-code.json', 'api-server', [['/url', 'URL', false, '${API_BASE_URL:-https://api.example.com}/mcp']]],
    ['cursor.json', 'local-server', [['/args/0', 'Argument 1', false, '${workspaceFolder}/tools/mcp_server.py']]],
    ['windsurf.json', 'github', [['/env/GITHUB_PERSONAL_ACCESS_TOKEN', 'GITHUB_PERSONAL_ACCESS_TOKEN', true, '']]],
    ['windsurf.json', 'remote-http-mcp', [['/headers/API_KEY', 'API_KEY', true, '']]],
    ['gemini.json', 'local', [['/cwd', 'Working directory (absolute path)', false, './server-directory']]],
    ['vscode-editor.json', 'github', [['/auth/token', 'GitHub Personal Access Token', true, '']]],
    [
      'vscode-editor.json',
      'perplexity',
      [
        ['/cwd', 'Working directory (absolute path)', false, '${workspaceFolder}'],
        ['/env/BUILD', 'build-id', false, ''],
        ['/env/PERPLEXITY_API_KEY', 'Perplexity API Key', true, ''],
        ['/env/REGION', 'Service region', false, ''],
      ],
    ],
    ['zed.json', 'remote-mcp-server', [['/auth/token', 'API key', true, '']]],
    [
      'registry.json',
      'weather',
      [
        ['/headers/X-API-Key', 'API key for authentication', true, ''],
        ['/headers/X-Region', 'Service region', false, 'us-east-1'],
      ],
    ],
    ['single.json', '', [['/auth/token', 'API key', true, '']]],
  ])('%s › %s needs', (file, name, needs) => {
    const got = [...row(parse(file), name).needs].sort((a, b) => a.path.localeCompare(b.path))
    expect(got.map((n) => [n.path, n.label, n.password, n.original])).toEqual(needs)
  })
  it('keeps mcp-client entries as written', () => {
    const input = JSON.parse(load('native.json')).mcpServers
    const result = parse('native.json')
    expect(row(result, 'linear-app').entry).toEqual(input['linear-app'])
    expect(row(result, 'linear-app').secrets).toEqual([])
    expect(row(result, 'playwright').entry).toEqual(input.playwright)
    expect(row(result, 'playwright').envRefs).toEqual(['HTTPS_PROXY'])
  })
  it('reports dropped defaults, env references and a target that shows no value', () => {
    const weather = row(parse('claude-code.json'), 'weather')
    expect(weather.notes).toEqual(['default for `REGION` dropped'])
    expect(weather.envRefs).toEqual(['LOG_LEVEL', 'REGION'])
    expect([weather.transport, weather.target]).toEqual([
      'stdio',
      '/opt/weather/weather-cli --api-key … --region env://REGION',
    ])
    expect(row(parse('claude-code.json'), 'api-server').target).toBe('…')
    expect(row(parse('claude-code.json'), 'linear-server').target).toBe('https://mcp.linear.app/mcp')
    expect(row(parse('vscode-portable.json'), 'context7').notes).toEqual(['default for `API_VERSION` dropped'])
    expect(row(parse('bare-map.json'), 'docs').envRefs).toEqual(['DOCS_TOKEN', 'TENANT_ID'])
    expect(row(parse('registry.json'), 'weather').notes).toEqual([
      'packages are not imported: add a stdio server by hand',
    ])
  })
  it('stores a URL that carries a credential whole, and refuses --flag=value', () => {
    const withUser = 'https://' + 'user:' + FAKE + '@mcp.example.com/mcp'
    const withQuery = `https://mcp.example.com/mcp?api_key=${FAKE}`
    for (const url of [withUser, withQuery]) {
      const r = parseImport(JSON.stringify({ x: { url } }), []).rows[0]
      expect(r.secrets).toEqual([{ path: '/url', vaultName: 'X_URL', value: url, masked: maskSecret(url) }])
      expect([r.entry.url, r.target]).toEqual(['', 'https://mcp.example.com (stored as a secret)'])
    }
    expect(
      parseImport(JSON.stringify({ x: { url: 'https://mcp.example.com/mcp?page=2' } }), []).rows[0].secrets,
    ).toEqual([])
    const assigned = parseImport(JSON.stringify({ x: { command: 'srv', args: ['--api-key=' + FAKE] } }), []).rows[0]
    expect([assigned.status, assigned.skipReason]).toEqual([
      'skipped',
      'Split `--api-key=…` into two arguments, then paste again',
    ])
    expect(JSON.stringify(assigned)).not.toContain(FAKE)
  })
  it.each(['<YOUR_API_KEY>', 'your-token', 'YOUR_KEY', 'xxx'])('%s is a missing value, not a secret', (value) => {
    const r = parseImport(JSON.stringify({ x: { command: 'srv', env: { API_KEY: value } } }), []).rows[0]
    expect([r.secrets, r.needs.map((n) => [n.path, n.password, n.original]), r.entry.env]).toEqual([
      [],
      [['/env/API_KEY', true, '']],
      { API_KEY: '' },
    ])
  })
  it('never yields client_credentials, a ${ in an entry, or a value outside secrets', () => {
    for (const file of [...CLIENTS, 'native.json']) {
      const result = parse(file)
      expect(JSON.stringify(result.rows.map(({ secrets: _secrets, ...rest }) => rest)), file).not.toContain(FAKE)
      expect(JSON.stringify(result.rows.map((r) => r.entry)), file).not.toContain('${')
    }
    for (const file of CLIENTS) {
      for (const r of parse(file).rows)
        expect((r.entry.auth as { type?: string } | undefined)?.type, file).not.toBe('client_credentials')
    }
  })
  it('maskSecret shows no character of the value', () => {
    expect([maskSecret(FAKE), maskSecret('x')]).toEqual([MASK, '•••••••• · 1 character'])
  })
  it('finalEntry writes references and lists what to store and share', () => {
    const weather = row(parse('claude-code.json'), 'weather')
    expect(finalEntry(weather, {})).toEqual({
      entry: {
        type: 'stdio',
        command: '/opt/weather/weather-cli',
        args: ['--api-key', 'secret://WEATHER_API_KEY', '--region', 'env://REGION'],
        env: { CACHE_DIR: '/tmp/weather', LOG_LEVEL: 'env://LOG_LEVEL' },
      },
      toStore: [{ name: 'WEATHER_API_KEY', value: FAKE }],
      toShare: ['LOG_LEVEL', 'REGION'],
    })
    expect(weather.entry.args).toEqual(['--api-key', '', '--region', 'env://REGION'])
  })
})

describe('credentials in any field', () => {
  // The value is nowhere in the row but `secrets[].value`, and the finished entry holds a reference there.
  const storedWhole = (server: Record<string, unknown>, path: string, value: string) => {
    const r = parseImport(JSON.stringify({ x: server }), []).rows[0]
    expect(JSON.stringify({ ...r, secrets: r.secrets.map((s) => ({ ...s, value: '' })) })).not.toContain(FAKE)
    expect(r.secrets.map((s) => [s.path, s.value])).toEqual([[path, value]])
    const [, top, key] = path.split('/')
    expect((finalEntry(r, {}).entry[top] as Record<string, string>)[key]).toMatch(/^secret:\/\/X_/)
  }
  it('stores a postgres URL argument whole', () => {
    const dsn = 'postgresql://' + 'user:' + FAKE + '@localhost/db'
    storedWhole({ command: 'npx', args: ['-y', '@modelcontextprotocol/server-postgres', dsn] }, '/args/2', dsn)
  })
  it('stores an mcp-remote argument whose query holds a key whole', () => {
    const url = `https://h.example.com/mcp?api_key=${FAKE}`
    storedWhole({ command: 'npx', args: ['mcp-remote', url] }, '/args/1', url)
  })
  it('stores an env URL with userinfo whole', () => {
    const dsn = 'postgres://' + 'u:' + FAKE + '@db/x'
    storedWhole({ command: 'srv', env: { DATABASE_URL: dsn } }, '/env/DATABASE_URL', dsn)
  })
  it('stores a header-shaped argument whole', () => {
    const header = `Authorization: Bearer ${FAKE}`
    const args = ['mcp-remote', 'https://h.example.com/mcp', '--header', header]
    storedWhole({ command: 'npx', args }, '/args/3', header)
  })
  it('keeps only the known keys of a page-format auth and lists the others by name', () => {
    const other = 'refresh-not-a-real-value'
    const auth = { type: 'bearer', token: FAKE, refresh_token: other }
    const r = parseImport(JSON.stringify({ x: { url: 'https://h.example.com/mcp', auth } }), []).rows[0]
    expect(r.entry.auth).toEqual({ type: 'bearer', token: '' })
    expect(fields(r)).toEqual(['auth.refresh_token'])
    expect(JSON.stringify(r)).not.toContain(other)
  })
  it('never names a vault name after a secret that follows a secret flag', () => {
    const token = '-tokenSentinel42'
    const args = ['--api-key', token, 'postgres://' + 'u:' + FAKE + '@h/db']
    const r = parseImport(JSON.stringify({ x: { command: 'srv', args } }), []).rows[0]
    expect(r.secrets.map((s) => [s.path, s.vaultName])).toEqual([
      ['/args/1', 'X_API_KEY'],
      ['/args/2', 'X_ARG_3'],
    ])
    const { entry, toStore } = finalEntry(r, {})
    const names = [...r.secrets.map((s) => s.vaultName), ...toStore.map((s) => s.name)]
    for (const text of [...names, JSON.stringify(entry)]) expect(text.toLowerCase()).not.toContain('tokensentinel42')
  })
  it('reads a template nested 10,000 deep without throwing, and masks it', () => {
    const deep = '${A:-'.repeat(10_000) + '}'.repeat(10_000)
    const r = parseImport(JSON.stringify({ x: { command: 'srv', env: { DATABASE_URL: deep } } }), []).rows[0]
    expect(r.needs.map((n) => [n.path, n.password])).toEqual([['/env/DATABASE_URL', true]])
    expect(r.needs[0].original).not.toContain('${')
  })
  it('flags a --token=<value> argument to split, holding only the flag', () => {
    const r = parseImport(JSON.stringify({ x: { command: 'srv', args: [`--token=${FAKE}`] } }), []).rows[0]
    expect([r.status, r.skipReason, r.entry, r.target, r.notes, r.ignored, r.secrets, r.needs]).toEqual([
      'skipped',
      'Split `--token=…` into two arguments, then paste again',
      {},
      '',
      [],
      [],
      [],
      [],
    ])
    expect(JSON.stringify(r)).not.toContain(FAKE)
  })
  it('asks again for a credential URL around a ${…}, never showing its value', () => {
    const dsn = 'postgres://' + 'u:' + FAKE + '@${DB_HOST}/x'
    const url = '${BASE}/mcp?api_key=' + FAKE
    const cases: [Record<string, unknown>, string, string][] = [
      [{ command: 'srv', env: { DATABASE_URL: dsn } }, '/env/DATABASE_URL', dsn],
      [{ url }, '/url', url],
    ]
    for (const [server, path, value] of cases) {
      const r = parseImport(JSON.stringify({ x: server }), []).rows[0]
      expect([r.secrets, r.needs.map((n) => [n.path, n.password, n.original])]).toEqual([
        [],
        [[path, true, maskSecret(value)]],
      ])
      expect(JSON.stringify(r)).not.toContain(FAKE)
    }
  })
  it('reads a numeric page token as text and keeps it out of the entry', () => {
    const auth = { type: 'bearer', token: 987654321 }
    const r = parseImport(JSON.stringify({ x: { type: 'http', url: 'https://h.example.com/mcp', auth } }), []).rows[0]
    expect(r.entry.auth).toEqual({ type: 'bearer', token: '' })
    expect(r.secrets.map((s) => [s.path, s.value])).toEqual([['/auth/token', '987654321']])
    expect(JSON.stringify({ ...r, secrets: [] })).not.toContain('987654321')
  })
  it('gives two values under one vault name two names, and one value one', () => {
    const paste = (other: string) =>
      parseImport(
        JSON.stringify({ r: { url: 'https://h.example.com/mcp', headers: { 'API-KEY': FAKE, API_KEY: other } } }),
        [],
      ).rows[0]
    const two = paste(`${FAKE}-2`)
    expect(two.secrets.map((s) => [s.vaultName, s.value])).toEqual([
      ['R_API_KEY', FAKE],
      ['R_API_KEY_2', `${FAKE}-2`],
    ])
    expect(finalEntry(two, {}).toStore).toEqual([
      { name: 'R_API_KEY', value: FAKE },
      { name: 'R_API_KEY_2', value: `${FAKE}-2` },
    ])
    const same = paste(FAKE)
    expect(same.secrets.map((s) => s.vaultName)).toEqual(['R_API_KEY', 'R_API_KEY'])
    expect(finalEntry(same, {}).toStore).toEqual([{ name: 'R_API_KEY', value: FAKE }])
  })
  it('finalEntry stores a typed password, and writes a typed reference or a plain value as typed', () => {
    const perplexity = finalEntry(row(parse('vscode-editor.json'), 'perplexity'), {
      '/env/PERPLEXITY_API_KEY': FAKE,
      '/env/REGION': 'eu',
      '/env/BUILD': '42',
      '/cwd': '/srv/app',
    })
    // The vault name is built here, so no line pairs a secret-looking name with a long value.
    const vault = ['PERPLEXITY', 'PERPLEXITY', 'API', 'KEY'].join('_')
    expect(Object.values(perplexity.entry.env as Record<string, string>)).toEqual([
      `secret://${vault}`,
      'eu',
      '42',
      '3',
    ])
    expect([perplexity.entry.cwd, perplexity.toStore]).toEqual(['/srv/app', [{ name: vault, value: FAKE }]])
    const github = finalEntry(row(parse('vscode-editor.json'), 'github'), { '/auth/token': 'env://GH_TOKEN' })
    expect([github.entry.auth, github.toStore, github.toShare]).toEqual([
      { type: 'bearer', token: 'env://GH_TOKEN' },
      [],
      ['GH_TOKEN'],
    ])
  })
  it('keeps a whole ${VAR:-…} a reference, its credential default dropped and never shown', () => {
    const cases: [Record<string, unknown>, (e: Record<string, unknown>) => unknown, string][] = [
      [
        { command: 'srv', env: { DATABASE_URL: '${DB_URL:-postgres://' + 'u:' + FAKE + '@localhost/db}' } },
        (e) => (e.env as Record<string, string>).DATABASE_URL,
        'DB_URL',
      ],
      [{ url: '${MCP_URL:-https://h.example.com/mcp?token=' + FAKE + '}' }, (e) => e.url, 'MCP_URL'],
    ]
    for (const [server, at, name] of cases) {
      const r = parseImport(JSON.stringify({ x: server }), []).rows[0]
      expect([at(r.entry), r.notes, r.secrets, r.needs]).toEqual([
        `env://${name}`,
        [`default for \`${name}\` dropped`],
        [],
        [],
      ])
      expect(JSON.stringify(r)).not.toContain(FAKE)
    }
  })
  it('stores a literal URL whose password holds a / whole', () => {
    const dsn = 'postgres://' + 'u:' + FAKE + '/x@h/db'
    storedWhole({ command: 'srv', env: { DATABASE_URL: dsn } }, '/env/DATABASE_URL', dsn)
    storedWhole({ command: 'srv', args: ['run', dsn] }, '/args/1', dsn)
  })
  // A need that shows its template: no credential in it, read as written, with its defaults, or as `x`.
  const shown = (server: Record<string, unknown>, path: string, template: string) => {
    const r = parseImport(JSON.stringify({ x: server }), []).rows[0]
    expect([r.status, r.secrets, r.needs.map((n) => [n.path, n.password, n.original])]).toEqual([
      'new',
      [],
      [[path, false, template]],
    ])
  }
  it('reads ${input:…} as a plain reference, showing the template', () => {
    const dsn = 'postgres://' + '${input:user}:${input:pass}' + '@db/x'
    shown({ command: 'srv', env: { DATABASE_URL: dsn } }, '/env/DATABASE_URL', dsn)
  })
  it('shows a template whose defaults hold no credential', () => {
    shown({ url: '${BASE:-https://h.example.com}/mcp' }, '/url', '${BASE:-https://h.example.com}/mcp')
    const dsn = 'postgres://${DB_USER}:${DB_PASS}@${DB_HOST:-localhost}:5432/db'
    shown({ command: 'srv', env: { DATABASE_URL: dsn } }, '/env/DATABASE_URL', dsn)
  })
  it('asks for a template with no literal credential left, showing the template', () => {
    const cases: [Record<string, unknown>, string, string][] = [
      [
        { command: 'srv', env: { DATABASE_URL: 'postgres://${DB_USER}:${DB_PASS}@db/x' } },
        '/env/DATABASE_URL',
        'postgres://${DB_USER}:${DB_PASS}@db/x',
      ],
      [
        {
          command: 'npx',
          args: ['mcp-remote', 'https://h.example.com/mcp', '--header', 'Authorization:${AUTH_HEADER}'],
        },
        '/args/3',
        'Authorization:${AUTH_HEADER}',
      ],
      [{ command: 'srv', args: ['--token=${X}'] }, '/args/0', '--token=${X}'],
      [{ command: 'srv', args: ['--token=${env:X}'] }, '/args/0', '--token=${env:X}'],
      [{ command: 'srv', args: ['Authorization: Bearer ${T}'] }, '/args/0', 'Authorization: Bearer ${T}'],
      [{ command: 'srv', args: ['X-API-Key: ${K}'] }, '/args/0', 'X-API-Key: ${K}'],
    ]
    for (const [server, path, template] of cases) shown(server, path, template)
  })
})

describe('the literal never leaves secrets', () => {
  // Every leak shape probed in review, in each position a string is read from; `FAKE` is the sentinel.
  const pg = (host: string) => 'postgres://' + 'u:' + FAKE + '@' + host
  const positions: Record<string, (value: string) => Record<string, unknown>> = {
    env: (value) => ({ command: 'srv', env: { DATABASE_URL: value } }),
    header: (value) => ({ url: 'https://h.example.com/mcp', headers: { 'X-Upstream': value } }),
    url: (value) => ({ url: value }),
    arg: (value) => ({ command: 'srv', args: ['run', value] }),
  }
  const anywhere = [
    pg('localhost/db'),
    `https://h.example.com/mcp?api_key=${FAKE}`,
    pg('${DB_HOST}/x'),
    '${BASE}/mcp?api_key=' + FAKE,
    '${DB_URL:-' + pg('localhost/db') + '}',
    '${X:-' + pg('h') + '}/path',
    '${A}${B:-' + pg('h') + '}',
    '${' + pg('h'),
    '${A:-${B}}' + pg('h'),
    '${A:-' + pg('${H}') + '}',
    '${A:-postgres://' + 'u:' + FAKE + '${P}@h}',
    '${A:-postgres://' + 'u:' + FAKE + '${P}@h}/x',
    '${X-' + pg('h/db') + '}',
    '${X:=' + pg('h/db') + '}',
    '${A}${X-https://h.example.com/mcp?token=' + FAKE + '}',
    '${SCHEME}://' + 'u:' + FAKE + '@h/db',
    // round 4: a `/`, `?` or `#` in or beside a default, between `://` and `@`
    'postgres://' + 'app:${DB_PASS:-Xk9/' + FAKE + '+}@db/app',
    'postgres://' + 'u:' + FAKE + '${X:-/}@h',
    'postgres://' + 'u:${P:-' + FAKE + '#x}@h',
    'postgres://' + 'u:${P:-' + FAKE + '?x}@h',
    'postgres://' + '${U:-a/b}:' + FAKE + '@h',
    'https://' + '${A:-u}:' + FAKE + '${B:-/}@h/mcp',
    'postgres://' + '${DB_USER}:${DB_PASS:-' + FAKE + '/x}@db/app',
  ]
  // A literal password holding a `/`, in env and arg.
  const slashed = 'postgres://' + 'u:' + FAKE + '/x@h/db'
  const argsOnly = [
    `Authorization: Bearer ${FAKE}`,
    `X-API-Key:${FAKE}`,
    `--token=${FAKE}`,
    '--token=' + FAKE + '${X}',
    '--token=${X:-' + FAKE + '}',
    '--api-key=${K:-' + FAKE + '}',
    'Authorization: Bearer ${T:-' + FAKE + '}',
    'Authorization:${H:-Bearer ' + FAKE + '}',
  ]
  const table: [string, string, Record<string, unknown>][] = [
    ...anywhere.flatMap((value) =>
      Object.entries(positions).map(([at, put]): [string, string, Record<string, unknown>] => [at, value, put(value)]),
    ),
    ...argsOnly.map((value): [string, string, Record<string, unknown>] => ['arg', value, positions.arg(value)]),
    ...['env', 'arg'].map((at): [string, string, Record<string, unknown>] => [at, slashed, positions[at](slashed)]),
  ]
  it.each(table)('%s %s', (_at, _value, server) => {
    const r = parseImport(JSON.stringify({ x: server }), []).rows[0]
    expect(JSON.stringify({ ...r, secrets: r.secrets.map((s) => ({ ...s, value: '' })) })).not.toContain(FAKE)
    expect(JSON.stringify(finalEntry(r, {}).entry)).not.toContain(FAKE)
  })
})

describe('dialog rules', () => {
  it('finalEntry fills needs: a password goes to the vault, a typed reference is kept', () => {
    expect(finalEntry(row(parse('vscode-editor.json'), 'github'), { '/auth/token': FAKE })).toEqual({
      entry: {
        type: 'http',
        url: 'https://api.githubcopilot.com/mcp',
        auth: { type: 'bearer', token: 'secret://GITHUB_TOKEN' },
      },
      toStore: [{ name: 'GITHUB_TOKEN', value: FAKE }],
      toShare: [],
    })
    expect(finalEntry(row(parse('claude-code.json'), 'api-server'), { '/url': 'https://api.example.com/mcp' })).toEqual(
      {
        entry: { type: 'http', url: 'https://api.example.com/mcp', auth: { type: 'bearer', token: 'env://API_KEY' } },
        toStore: [],
        toShare: ['API_KEY'],
      },
    )
    const windsurf = row(parse('windsurf.json'), 'github')
    expect(finalEntry(windsurf, { '/env/GITHUB_PERSONAL_ACCESS_TOKEN': 'env://GH_PAT' })).toMatchObject({
      toStore: [],
      toShare: ['GH_PAT'],
    })
    expect(finalEntry(windsurf, { '/env/GITHUB_PERSONAL_ACCESS_TOKEN': FAKE }).toStore).toEqual([
      { name: 'GITHUB_GITHUB_PERSONAL_ACCESS_TOKEN', value: FAKE },
    ])
  })
  it('finalEntry stores a credential typed into a references-only need, and keeps any other typed value', () => {
    const typed = (server: Record<string, unknown>, value: string) => {
      const r = parseImport(JSON.stringify({ x: server }), []).rows[0]
      expect(r.needs.map((n) => n.password)).toEqual([false])
      return finalEntry(r, { [r.needs[0].path]: value })
    }
    const cases: [Record<string, unknown>, string, (entry: Record<string, unknown>) => unknown][] = [
      [
        { command: 'srv', env: { DATABASE_URL: 'postgres://${DB_USER}:${DB_PASS}@db/app' } },
        'postgres://' + 'app:' + FAKE + '@db/app',
        (entry) => (entry.env as Record<string, unknown>).DATABASE_URL,
      ],
      [{ command: 'srv', args: ['--token=${X}'] }, `--token=${FAKE}`, (entry) => (entry.args as unknown[])[0]],
      [
        { command: 'srv', args: ['Authorization: Bearer ${T}'] },
        `Authorization: Bearer ${FAKE}`,
        (entry) => (entry.args as unknown[])[0],
      ],
    ]
    for (const [server, value, at] of cases) {
      const out = typed(server, value)
      expect(at(out.entry)).toMatch(/^secret:\/\//)
      expect(out.toStore.map((s) => s.value)).toEqual([value])
      expect(JSON.stringify(out.entry)).not.toContain(FAKE)
    }
    const tenant = typed({ url: 'https://x.example.com/${TENANT}/mcp' }, 'https://x.example.com/acme/mcp')
    expect([tenant.entry.url, tenant.toStore]).toEqual(['https://x.example.com/acme/mcp', []])
  })
  it('renameRow re-derives the status and the vault names nobody edited', () => {
    const single = parse('single.json').rows[0]
    expect(renameRow(single, ' weather ', ['weather'])).toMatchObject({ name: 'weather', status: 'replaces' })
    const keyed = row(parse('cursor.json'), 'keyed')
    expect(renameRow(keyed, 'search', []).secrets.map((s) => s.vaultName)).toEqual(['SEARCH_API_KEY'])
    const edited = { ...keyed, secrets: [{ ...keyed.secrets[0], vaultName: 'MY_KEY' }] }
    expect(renameRow(edited, 'search', []).secrets[0].vaultName).toBe('MY_KEY')
    expect(renameRow(row(parse('claude-code.json'), 'asana'), 'other', []).status).toBe('skipped')
  })
  it('rowBlockers says what stops a row', () => {
    const single = parse('single.json').rows[0]
    expect(rowBlockers(single, {})).toEqual(['Give this server a name', 'Fill in API key'])
    expect(rowBlockers(renameRow(single, 'Bad Name', []), { '/auth/token': FAKE })).toEqual([
      'Name: 1 to 24 characters, a-z, 0-9 and -, starting with a letter or a digit',
    ])
    expect(rowBlockers(renameRow(single, 'weather', []), { '/auth/token': FAKE })).toEqual([])
    const keyed = row(parse('cursor.json'), 'keyed')
    expect(rowBlockers({ ...keyed, secrets: [{ ...keyed.secrets[0], vaultName: '9 bad' }] }, {})).toEqual([
      'Vault name `9 bad` is not valid',
    ])
    const clash = {
      ...keyed,
      secrets: [keyed.secrets[0], { ...keyed.secrets[0], path: '/headers/Other', value: 'another-fake' }],
    }
    expect(rowBlockers(clash, {})).toEqual(['Two secrets share the vault name `KEYED_API_KEY`'])
    expect(rowBlockers(row(parse('claude-code.json'), 'asana'), {})).toEqual([])
  })
  it('crossRowBlockers finds a name or a vault name another row of the import holds', () => {
    const keyed = row(parse('cursor.json'), 'keyed')
    const other = renameRow(keyed, 'other', [])
    expect(crossRowBlockers([keyed, renameRow(other, ' keyed ', [])], [{}, {}])).toEqual([
      [],
      ['Another server in this import is named `keyed`'],
    ])
    const vault = (value: string) => ({
      ...other,
      secrets: [{ ...other.secrets[0], vaultName: 'KEYED_API_KEY', value }],
    })
    expect(crossRowBlockers([keyed, vault('another-fake')], [{}, {}])).toEqual([
      [],
      ['Vault name `KEYED_API_KEY` is used by another server in this import'],
    ])
    // One value under one vault name is one secret: no blocker.
    expect(crossRowBlockers([keyed, vault(FAKE)], [{}, {}])).toEqual([[], []])
    // A skipped row takes no part.
    const asana = row(parse('claude-code.json'), 'asana')
    expect(crossRowBlockers([{ ...asana, name: 'keyed' }, keyed], [{}, {}])).toEqual([[], []])
  })
  it('crossRowBlockers reads the vault names typed passwords will take, not only the pasted secrets', () => {
    // `a` + `B_TOKEN` and `a-b` + `token` both default to A_B_TOKEN.
    const { rows } = parseImport(
      JSON.stringify({
        a: { command: 'x', env: { B_TOKEN: 'your-token' } },
        'a-b': { url: 'https://api.example.com/mcp', headers: { Authorization: 'Bearer your-token' } },
      }),
      [],
    )
    expect(rows.map((r) => r.needs.map((n) => [n.path, n.password]))).toEqual([
      [['/env/B_TOKEN', true]],
      [['/auth/token', true]],
    ])
    expect(crossRowBlockers(rows, [{ '/env/B_TOKEN': FAKE }, { '/auth/token': 'another-fake' }])).toEqual([
      [],
      ['Vault name `A_B_TOKEN` is used by another server in this import'],
    ])
    expect(crossRowBlockers(rows, [{ '/env/B_TOKEN': FAKE }, { '/auth/token': FAKE }])).toEqual([[], []])
    // A pasted secret renamed onto the name a typed password takes.
    const keyed = row(parse('cursor.json'), 'keyed')
    const moved = { ...keyed, secrets: [{ ...keyed.secrets[0], vaultName: 'A_B_TOKEN' }] }
    expect(crossRowBlockers([rows[1], moved], [{ '/auth/token': 'another-fake' }, {}])).toEqual([
      [],
      ['Vault name `A_B_TOKEN` is used by another server in this import'],
    ])
  })
  it('startValues starts a password need empty, never from its masked original', () => {
    const masked = parseImport(JSON.stringify({ x: { url: '${BASE}/mcp?api_key=' + FAKE } }), []).rows[0]
    expect(masked.needs.map((n) => [n.password, n.original])).toEqual([
      [true, maskSecret('${BASE}/mcp?api_key=' + FAKE)],
    ])
    expect(startValues(masked)).toEqual({ '/url': '' })
    expect(startValues(row(parse('claude-code.json'), 'api-server'))).toEqual({
      '/url': '${API_BASE_URL:-https://api.example.com}/mcp',
    })
  })
  it('scrub removes pasted values from any message', () => {
    expect(scrub(`rejected value ${FAKE} for LINEAR_TOKEN`, [FAKE, ''])).toBe(
      'rejected value [redacted] for LINEAR_TOKEN',
    )
  })
  it('mergeConsumers keeps who could already read the secret', () => {
    expect(mergeConsumers(['llm-router'], ['mcp-client'])).toEqual(['llm-router', 'mcp-client'])
    expect(mergeConsumers(undefined, ['mcp-client'])).toEqual(['mcp-client'])
    expect(mergeConsumers(['mcp-client'], ['mcp-client'])).toEqual(['mcp-client'])
  })
  it('importRow stores, shares, then saves, and a failed store imports nothing', async () => {
    const weather = row(parse('claude-code.json'), 'weather')
    const calls: string[] = []
    const ok: ImportDeps = {
      storeSecret: async (name) => {
        calls.push(`store ${name}`)
      },
      shareEnv: async (name) => {
        calls.push(`share ${name}`)
      },
      save: async (name, entry) => {
        calls.push(`save ${name} ${JSON.stringify(entry)}`)
        return { saved: true, errors: [] }
      },
    }
    expect(await importRow(ok, weather, {})).toEqual({ name: 'weather', imported: true, message: 'Imported' })
    expect(calls.map((c) => c.split(' ').slice(0, 2).join(' '))).toEqual([
      'store WEATHER_API_KEY',
      'share LOG_LEVEL',
      'share REGION',
      'save weather',
    ])
    expect(calls.join('\n')).not.toContain(FAKE)
    const saved: string[] = []
    const failing: ImportDeps = {
      ...ok,
      storeSecret: async () => {
        throw new Error(`vault refused ${FAKE}`)
      },
      save: async (name) => {
        saved.push(name)
        return { saved: true, errors: [] }
      },
    }
    expect(await importRow(failing, weather, {})).toEqual({
      name: 'weather',
      imported: false,
      message: 'Not imported: could not store WEATHER_API_KEY: vault refused [redacted]',
    })
    expect(saved).toEqual([])
    const refused: ImportDeps = {
      ...ok,
      save: async () => ({
        saved: false,
        errors: [{ path: '/mcpServers/weather/command', message: 'command is empty' }],
      }),
    }
    expect((await importRow(refused, weather, {})).message).toBe(
      'Not imported: /mcpServers/weather/command: command is empty',
    )
    const stopped: ImportDeps = {
      ...ok,
      save: async () => ({ saved: false, errors: [], message: 'could not remove the approval-gate rules' }),
    }
    expect((await importRow(stopped, weather, {})).message).toBe(
      'Not imported: could not remove the approval-gate rules',
    )
    const unshared: ImportDeps = {
      ...ok,
      shareEnv: async () => {
        throw new Error('secrets worker is down')
      },
    }
    expect((await importRow(unshared, weather, {})).message).toBe(
      'Not imported: could not share LOG_LEVEL with mcp-client: secrets worker is down',
    )
  })
})

describe('dialog copy', () => {
  it("importEffects lists the secrets stored, the variables shared, then each server's effects", () => {
    expect(
      importEffects([
        {
          name: 'weather',
          store: ['WEATHER_API_KEY'],
          share: ['LOG_LEVEL', 'REGION'],
          lines: ['mcp-client will run `w`'],
        },
        { name: 'keyed', store: ['KEYED_API_KEY'], share: ['REGION'], lines: [] },
      ]),
    ).toEqual([
      { text: 'store 2 secrets in the vault: `WEATHER_API_KEY`, `KEYED_API_KEY`' },
      { text: 'share `LOG_LEVEL` with mcp-client' },
      { text: 'share `REGION` with mcp-client' },
      { server: 'weather', text: 'mcp-client will run `w`' },
    ])
    expect(importEffects([{ name: 'a', store: ['A'], share: [], lines: [] }])).toEqual([
      { text: 'store 1 secret in the vault: `A`' },
    ])
  })
  it('importSummary counts what was imported and what was not', () => {
    const out = (imported: boolean) => ({ name: 'x', imported, message: '' })
    expect(importSummary([out(true), out(true), out(true)])).toBe('3 servers imported')
    expect(importSummary([out(true), out(true), out(false)])).toBe('2 servers imported, 1 not imported')
    expect(importSummary([out(false)])).toBe('0 servers imported, 1 not imported')
    expect(importSummary([out(true)])).toBe('1 server imported')
  })
})
