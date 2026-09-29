import assert from 'node:assert/strict'
import { test } from 'node:test'
import { addRepository, normalize, problems, setPath } from './form-model.js'

test('a stored value keeps the keys this form never renders', () => {
  const stored = { enabled: false, ingest: { breaker_failures: 9 }, service_aliases: { a: 'b' } }
  const config = normalize(stored)
  assert.equal(config.enabled, false)
  assert.deepEqual(config.ingest, { breaker_failures: 9 })
  assert.deepEqual(config.service_aliases, { a: 'b' })
  assert.equal(config.retention.cron, '0 0 3 * * *', 'and gains the defaults it lacked')
})

test('setting one field leaves its siblings alone', () => {
  const config = normalize({ sources: { log: { enabled: true, join_window_ms: 5000 } } })
  const next = setPath(config, 'sources.log.enabled', false)
  assert.equal(next.sources.log.enabled, false)
  assert.equal(next.sources.log.join_window_ms, 5000)
  assert.equal(next.sources.trace.enabled, true)
  assert.equal(config.sources.log.enabled, true, 'and does not mutate the original')
})

test('a relative path is refused before the round trip', () => {
  const config = normalize({ projects: [{ id: 'x', path: 'relative/thing', workers: [] }] })
  assert.deepEqual(problems(config), ['x needs an absolute path'])
})

test('a worker cannot live in two checkouts', () => {
  const config = normalize({
    projects: [
      { id: 'a', path: '/a', workers: ['harness'] },
      { id: 'b', path: '/b', workers: ['harness'] },
    ],
  })
  assert.deepEqual(problems(config), ['harness is mapped to both a and b'])
})

test('the prune schedule must be a cron expression', () => {
  assert.deepEqual(problems(normalize({ retention: { cron: '0 3 * * *' } })), [
    'the prune schedule needs a six or seven field cron expression',
  ])
  assert.deepEqual(problems(normalize({ retention: { cron: '0 0 3 * * *' } })), [])
})

test('keeping nothing is not a retention policy', () => {
  assert.deepEqual(problems(normalize({ retention: { evidence_per_group: 0 } })), [
    'keep at least one bundle per group',
  ])
})

test('a second checkout with the same folder name gets its own id', () => {
  let config = normalize({})
  config = addRepository(config, '/home/dev/workers')
  config = addRepository(config, '/other/workers')
  assert.deepEqual(
    config.projects.map((repository) => repository.id),
    ['workers', 'workers-2'],
  )
})

import { repositoryAt, setRepositoryPath, setRepositoryWorkers } from './form-model.js'

const mapped = {
  projects: [
    { id: 'workers', path: '/w', workers: ['harness', 'queue'] },
    { id: 'iii', path: '/iii/', workers: ['iii'] },
  ],
}

test('picking a folder that is already mapped finds that repository', () => {
  assert.equal(repositoryAt(mapped, '/iii')?.id, 'iii', 'a trailing slash is the same folder')
  assert.equal(repositoryAt(mapped, '/w/')?.id, 'workers')
  assert.equal(repositoryAt(mapped, '/elsewhere'), undefined)
})

test('a repository can move to another folder and keep its workers', () => {
  const next = setRepositoryPath(mapped, 'workers', '/home/w')
  assert.deepEqual(next.projects[0], { id: 'workers', path: '/home/w', workers: ['harness', 'queue'] })
  assert.deepEqual(next.projects[1], mapped.projects[1])
})

test('a worker given to one repository leaves the other', () => {
  const next = setRepositoryWorkers(mapped, 'iii', ['iii', 'queue', ' queue '])
  assert.deepEqual(next.projects[1].workers, ['iii', 'queue'], 'trimmed, once')
  assert.deepEqual(next.projects[0].workers, ['harness'], 'queue moved, it is not mapped twice')
})

test('a value stored under the former key is read, and saved under the new one only', () => {
  const config = normalize({ repositories: [{ id: 'workers', path: '/w', workers: ['harness'] }], enabled: true })
  assert.deepEqual(config.projects, [{ id: 'workers', path: '/w', workers: ['harness'] }])
  assert.equal('repositories' in config, false, 'the worker refuses both keys together')
  assert.deepEqual(
    normalize({ projects: [{ id: 'a', path: '/a', workers: [] }], repositories: [{ id: 'b', path: '/b', workers: [] }] }).projects,
    [{ id: 'a', path: '/a', workers: [] }],
    'the new key wins when both are stored',
  )
})
