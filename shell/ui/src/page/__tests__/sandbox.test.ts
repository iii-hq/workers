import { describe, expect, it } from 'vitest'
import {
  buildGuestTree,
  catPayload,
  escapeRegex,
  type FsEntry,
  grepPayload,
  isFunctionNotFound,
  lsPayload,
  parseFleetEvent,
  parseSandboxList,
  sandboxGone,
  sandboxTarget,
  statPayload,
} from '../sandbox'

const entry = (name: string, kind: 'file' | 'dir' | 'symlink' = 'file'): FsEntry => ({
  name,
  is_dir: kind === 'dir',
  is_symlink: kind === 'symlink',
  size: 0,
})

describe('payload builders', () => {
  const target = sandboxTarget('3f2a4c1d-0000-0000-0000-000000000000')

  it('omit the target key entirely for host (the wire default)', () => {
    // An explicit `{kind:"host"}` would be fine, but `target: null` is a
    // server-side deserialize error — omission is the one safe spelling.
    for (const payload of [
      lsPayload('/x', null),
      statPayload('/x', null),
      grepPayload({ path: '/x', pattern: 'p', ignoreCase: false }, null),
      catPayload('/x', null),
    ]) {
      expect('target' in payload).toBe(false)
    }
  })

  it('carry the sandbox target verbatim when selected', () => {
    for (const payload of [
      lsPayload('/x', target),
      statPayload('/x', target),
      grepPayload({ path: '/x', pattern: 'p', ignoreCase: true }, target),
      catPayload('/x', target),
    ]) {
      expect(payload.target).toEqual({
        kind: 'sandbox',
        sandbox_id: '3f2a4c1d-0000-0000-0000-000000000000',
      })
    }
  })

  it('cat rides argv form with no cwd/env/stdin (host-only fields, S210 on sandbox)', () => {
    const payload = catPayload('/etc/os release', target)
    expect(payload.command).toBe('cat')
    expect(payload.args).toEqual(['/etc/os release'])
    for (const key of ['cwd', 'env', 'stdin']) {
      expect(key in payload).toBe(false)
    }
  })
})

describe('parseSandboxList', () => {
  it('tolerates junk: non-object input, rows without sandbox_id', () => {
    expect(parseSandboxList(null)).toEqual([])
    expect(parseSandboxList('nope')).toEqual([])
    expect(parseSandboxList({ sandboxes: 'nope' })).toEqual([])
    expect(
      parseSandboxList({ sandboxes: [{ name: 'x' }, null, 42, { sandbox_id: '' }] }),
    ).toEqual([])
  })

  it('defaults every missing field and keeps stopped honest', () => {
    const out = parseSandboxList({
      sandboxes: [
        { sandbox_id: 'a' },
        { sandbox_id: 'b', name: 'runner', image: 'img', stopped: true },
      ],
    })
    expect(out).toEqual([
      { sandbox_id: 'a', name: null, image: '', stopped: false },
      { sandbox_id: 'b', name: 'runner', image: 'img', stopped: true },
    ])
  })
})

describe('parseFleetEvent', () => {
  it('applies only { kind: "fleet" } snapshots', () => {
    expect(parseFleetEvent(null)).toBeNull()
    expect(parseFleetEvent({ kind: 'exec', sandboxes: [] })).toBeNull()
    expect(parseFleetEvent({ sandboxes: [{ sandbox_id: 'a' }] })).toBeNull()
  })

  it('carries the parsed fleet and the daemon-absent flag', () => {
    const snapshot = parseFleetEvent({
      kind: 'fleet',
      sandboxes: [{ sandbox_id: 'a' }],
      daemon_absent: true,
    })
    expect(snapshot?.sandboxes.map((s) => s.sandbox_id)).toEqual(['a'])
    expect(snapshot?.daemonAbsent).toBe(true)
    expect(parseFleetEvent({ kind: 'fleet', sandboxes: [] })?.daemonAbsent).toBe(false)
  })
})

describe('isFunctionNotFound', () => {
  it('matches the bus-level absence phrasings', () => {
    expect(isFunctionNotFound('Function sandbox::list not found')).toBe(true)
    expect(isFunctionNotFound('function_not_found')).toBe(true)
    expect(isFunctionNotFound('unknown function sandbox::list')).toBe(true)
    expect(isFunctionNotFound('no such function')).toBe(true)
  })

  it('ignores prose that merely contains "not found" (a missing path must not hide the picker)', () => {
    expect(isFunctionNotFound('path /tmp/x not found')).toBe(false)
    expect(isFunctionNotFound('S211: no such file or directory')).toBe(false)
  })
})

describe('sandboxGone', () => {
  const fleet = (
    status: 'loading' | 'ready' | 'absent' | 'error',
    ids: string[] = [],
  ) => ({
    status,
    sandboxes: ids.map((sandbox_id) => ({
      sandbox_id,
      name: null,
      image: '',
      stopped: false,
    })),
  })

  it('host mode never degrades', () => {
    expect(sandboxGone(null, fleet('ready'))).toBe(false)
    expect(sandboxGone(null, fleet('absent'))).toBe(false)
  })

  it('a selected sandbox missing from a ready fleet is gone', () => {
    expect(sandboxGone('a', fleet('ready', ['a', 'b']))).toBe(false)
    expect(sandboxGone('a', fleet('ready', ['b']))).toBe(true)
  })

  it('a vanished daemon takes every sandbox with it', () => {
    expect(sandboxGone('a', fleet('absent', ['a']))).toBe(true)
  })

  it('transient states never evict an open view', () => {
    expect(sandboxGone('a', fleet('loading'))).toBe(false)
    expect(sandboxGone('a', fleet('error'))).toBe(false)
  })
})

describe('buildGuestTree', () => {
  it('marks directories with the trailing slash and keys kinds slash-less', () => {
    const loaded = new Map<string, FsEntry[]>([
      ['', [entry('usr', 'dir'), entry('init'), entry('lib64', 'symlink')]],
      ['usr', [entry('bin', 'dir')]],
    ])
    const tree = buildGuestTree(loaded)
    expect(tree.paths).toEqual(['init', 'lib64', 'usr/', 'usr/bin/'])
    expect(tree.kinds.get('usr')).toBe('dir')
    expect(tree.kinds.get('usr/bin')).toBe('dir')
    expect(tree.kinds.get('init')).toBe('file')
    expect(tree.kinds.get('lib64')).toBe('symlink')
    expect(tree.truncations).toEqual([])
  })

  it('renders an unlisted directory as expandable via its parent entry', () => {
    const tree = buildGuestTree(new Map([['', [entry('etc', 'dir')]]]))
    expect(tree.paths).toEqual(['etc/'])
    expect(tree.kinds.get('etc')).toBe('dir')
  })

  it('returns the empty tree before the root listing lands', () => {
    const tree = buildGuestTree(new Map())
    expect(tree.paths).toEqual([])
  })
})

describe('escapeRegex', () => {
  it('neutralizes every metacharacter for literal grep queries', () => {
    const literal = 'a.b*c?(d)[e]{f}|g^h$i\\j+k'
    const re = new RegExp(escapeRegex(literal))
    expect(re.test(literal)).toBe(true)
    expect(re.test('aXb*c?(d)[e]{f}|g^h$i\\j+k')).toBe(false)
  })
})
