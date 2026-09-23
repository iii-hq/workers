import { describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import { messageFileDirectories, resolveChatFile } from './file-navigation'

const message = (id: string): Message => ({
  id, role: 'assistant', content: 'reference', createdAt: 0,
})
const scope = (id: string, path: string | null, previousPath?: string | null): Message => ({
  id, role: 'system', kind: 'working-dir', createdAt: 0, content: '',
  scope: { path, previousPath, cause: 'selected' },
})

describe('historical file directories', () => {
  it('uses both sides of durable changes, even in paged history', () => {
    const messages = [message('old'), scope('change', '/new', '/old'), message('new')]
    const dirs = messageFileDirectories(messages, '/other')
    expect(dirs.get('old')).toEqual({ path: '/old', recorded: true })
    expect(dirs.get('new')).toEqual({ path: '/new', recorded: true })
  })
  it('never substitutes the current folder for an explicitly unscoped interval', () => {
    const dirs = messageFileDirectories([
      message('unknown'), scope('one', '/repo'), message('scoped'),
      scope('two', null, '/repo'), message('unscoped'),
    ], '/current')
    expect(dirs.get('unknown')?.path).toBeNull()
    expect(dirs.get('scoped')?.path).toBe('/repo')
    expect(dirs.get('unscoped')?.path).toBeNull()
  })
  it('marks legacy or compacted history as unproven instead of claiming provenance', () => {
    expect(messageFileDirectories([message('legacy')], '/current').get('legacy'))
      .toEqual({ path: '/current', recorded: false })
  })
})

describe('resolveChatFile', () => {
  it('accepts absolute files without a workspace and leaves canonicalization to the worker', () => {
    expect(resolveChatFile({ path: '/other/file.ts' }, null)).toBe('/other/file.ts')
    expect(resolveChatFile({ path: '../symlink/file.ts' }, '/repo/')).toBe('/repo/../symlink/file.ts')
    expect(resolveChatFile({ path: 'a#b?.ts' }, '/repo')).toBe('/repo/a#b?.ts')
  })
  it.each(['', 'src/', '//host/a', 'C:\\repo\\a.ts', 'javascript:x', 'src/\0a'])('rejects %s', (path) => {
    expect(() => resolveChatFile({ path }, '/repo')).toThrow()
  })
  it('rejects unknown directories and invalid numeric coordinates', () => {
    expect(() => resolveChatFile({ path: 'a.ts' }, null)).toThrow('original folder')
    expect(() => resolveChatFile({ path: 'a.ts' }, 'relative')).toThrow('original folder')
    expect(() => resolveChatFile({ path: 'a.ts', range: { from: 0, to: 4 } }, '/repo')).toThrow('invalid')
    expect(() => resolveChatFile({ path: 'a.ts', range: { from: 1, to: Infinity } }, '/repo')).toThrow('invalid')
  })
})
