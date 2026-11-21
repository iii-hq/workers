import { describe, expect, it } from 'vitest'
import { looksSecretName } from '../src/secret-names'

describe('looksSecretName', () => {
  // The same vector as `looks_secret_name` in mcp-client/src/refs.rs.
  it('flags header and env names that probably carry a credential', () => {
    for (const name of [
      'X-API-Key',
      'GITHUB_TOKEN',
      'Authorization',
      'cookie',
      'apikey',
      'MY_PAT',
      'DB_PASSWD',
      'X-Auth-Token',
      'CREDENTIALS_FILE',
      'proxy-authorization',
    ]) {
      expect(looksSecretName(name), name).toBe(true)
    }
    for (const name of ['X-Workspace', 'PLAYWRIGHT_BROWSERS_PATH', 'HTTPS_PROXY', 'MONKEY', 'AUTHOR']) {
      expect(looksSecretName(name), name).toBe(false)
    }
  })
})
