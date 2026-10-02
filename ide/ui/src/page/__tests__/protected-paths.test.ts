import { describe, expect, it } from 'vitest'
import { isProtectedPath } from '../protected-paths'

const DEFAULTS = ['**/.env', '**/.env.*', '**/*.pem', '**/*.key', '**/secrets/**']

describe('isProtectedPath', () => {
  it("matches the worker's default protected paths, at the top or below", () => {
    for (const path of [
      '.env',
      'dev/.env',
      '.env.local',
      'a/b/server.pem',
      'id.key',
      'secrets/token',
      'x/secrets/y/z',
    ]) {
      expect(isProtectedPath(path, DEFAULTS)).toBe(true)
    }
  })

  it('leaves look-alikes alone', () => {
    for (const path of ['.envrc', 'env', 'a/.environment', 'keys.md', 'secret/x', 'pem.txt']) {
      expect(isProtectedPath(path, DEFAULTS)).toBe(false)
    }
    expect(isProtectedPath(null, DEFAULTS)).toBe(false)
    expect(isProtectedPath('.env', [])).toBe(false)
  })
})
