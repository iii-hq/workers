// biome-ignore-all lint/suspicious/noTemplateCurlyInString: pasted configs spell references `${VAR}`
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { type ImportResult, type ImportRow, parseImport, slugName } from '../src/importer'

// Fixtures hold the marker `@@FAKE@@` where a pasted secret goes, so no file holds a credential shape.
const FAKE = 'test-not-a-real-token'
const MASK = '•••••••• · 21 characters'
const COMMON = 'Claude Code, Cursor and similar'
const load = (name: string) =>
  readFileSync(new URL(`./fixtures/import/${name}`, import.meta.url), 'utf8').replaceAll('@@FAKE@@', FAKE)
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
