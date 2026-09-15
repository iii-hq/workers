import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { MAX_EMAIL_LENGTH, isEmailish } from '../src/subscribe.mjs'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

test('the signup box only forwards something shaped like one address', () => {
  assert.ok(isEmailish('someone@example.com'))
  assert.ok(isEmailish('  someone@example.co.uk  '), 'surrounding space is trimmed')

  for (const value of [
    '',
    '   ',
    'nope',
    'someone@example',
    'someone.example.com',
    'two addresses@example.com, other@example.com',
    'someone@exam ple.com',
    undefined,
    null,
    42,
    { email: 'someone@example.com' },
  ]) {
    assert.equal(isEmailish(value), false, `accepted ${JSON.stringify(value)}`)
  }

  // A pasted paragraph must not become a POST to someone else's service.
  const long = `${'a'.repeat(MAX_EMAIL_LENGTH)}@example.com`
  assert.equal(isEmailish(long), false, 'accepted an address over the RFC ceiling')
})

test('the signup request carries a deadline', () => {
  // The worker registers against a live engine at import, so the wiring is
  // read rather than called: what matters is that no fetch here can hang.
  const source = readFileSync(join(root, 'src', 'index.mjs'), 'utf8')
  assert.match(source, /signal: AbortSignal\.timeout\(SIGNUP_TIMEOUT_MS\)/)
  assert.match(source, /const SIGNUP_TIMEOUT_MS = [\d_]+/)
  assert.equal(source.match(/await fetch\(/g)?.length, 1, 'a second fetch needs its own deadline')
})

test('progress is written with atomic ops, never read-then-replace', () => {
  const source = readFileSync(join(root, 'src', 'index.mjs'), 'utf8')
  assert.match(source, /function_id: 'state::update'/)
  assert.doesNotMatch(source, /function_id: 'state::set'/, 'a set() call can lose an interleaved write')
})
