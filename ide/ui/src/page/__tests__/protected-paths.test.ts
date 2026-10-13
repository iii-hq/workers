import { describe, expect, it } from 'vitest'
import { isProtectedPath, workerRelative } from '../protected-paths'

const DEFAULTS = ['**/.env', '**/.env.*', '**/*.pem', '**/*.key', '**/secrets/**']
const BASES = ['/repo']

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
      expect(isProtectedPath(path, DEFAULTS, '/repo', BASES)).toBe(true)
    }
  })

  it('leaves look-alikes alone', () => {
    for (const path of ['.envrc', 'env', 'a/.environment', 'keys.md', 'secret/x', 'pem.txt']) {
      expect(isProtectedPath(path, DEFAULTS, '/repo', BASES)).toBe(false)
    }
    expect(isProtectedPath(null, DEFAULTS, '/repo', BASES)).toBe(false)
    expect(isProtectedPath('.env', [], '/repo', BASES)).toBe(false)
  })

  it('matches an anchored glob from the base path, not from the IDE folder below it', () => {
    expect(isProtectedPath('private.env', ['config/private.env'], '/repo/config', BASES)).toBe(true)
    expect(isProtectedPath('private.env', ['config/private.env'], '/other', BASES)).toBe(false)
  })
})

describe('workerRelative', () => {
  it('is relative to the first base path holding it, else the absolute path unrooted', () => {
    expect(workerRelative('/repo/a/.env', ['/tmp', '/repo'])).toBe('a/.env')
    expect(workerRelative('/home/me/dev/.env', ['/repo'])).toBe('home/me/dev/.env')
    expect(workerRelative('/repository/x', ['/repo'])).toBe('repository/x')
  })
})
