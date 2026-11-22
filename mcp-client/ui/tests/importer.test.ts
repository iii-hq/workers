// biome-ignore-all lint/suspicious/noTemplateCurlyInString: pasted configs spell references `${VAR}`
import { readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { finalEntry, type ImportResult, type ImportRow, maskSecret, parseImport, slugName } from '../src/importer'

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
  it('stores a URL that carries a credential whole', () => {
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
  it('stores a --token=<value> argument whole', () => {
    storedWhole({ command: 'srv', args: [`--token=${FAKE}`] }, '/args/0', `--token=${FAKE}`)
  })
  it('asks again for a credential URL around a ${…}, never showing its value', () => {
    const cases: [Record<string, unknown>, string][] = [
      [{ command: 'srv', env: { DATABASE_URL: 'postgres://' + 'u:' + FAKE + '@${DB_HOST}/x' } }, '/env/DATABASE_URL'],
      [{ url: '${BASE}/mcp?api_key=' + FAKE }, '/url'],
    ]
    for (const [server, path] of cases) {
      const r = parseImport(JSON.stringify({ x: server }), []).rows[0]
      expect([r.secrets, r.needs.map((n) => [n.path, n.password])]).toEqual([[], [[path, true]]])
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
})
