import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  DEFAULT_SIGNUP_URL,
  errorCode,
  isEmailish,
  isSignupUrlAllowed,
  MAX_EMAIL_LENGTH,
  SIGNUP_SOURCE,
  signupOutcome,
} from '../src/subscribe.mjs'

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

test('a signup is remembered as a time on the progress, never as the address', () => {
  // Read, not called (the worker registers against a live engine at import):
  // the page shows "You are on the list." from `subscribed_at`.
  const source = readFileSync(join(root, 'src', 'index.mjs'), 'utf8')
  const handler = source.slice(source.indexOf("'learn-iii::subscribe'"), source.indexOf('// Injected console UI'))
  assert.match(handler, /merge', value: \{ subscribed_at: at, updated_at: at \}/)
  assert.doesNotMatch(handler, /stateUpdate\([^)]*email/s, 'the address must not reach state')
})

test('the signup goes to iii.dev as source ade, with no Origin of our own', () => {
  assert.equal(DEFAULT_SIGNUP_URL, 'https://iii.dev/api/subscribe')
  assert.equal(SIGNUP_SOURCE, 'ade')
  const source = readFileSync(join(root, 'src', 'index.mjs'), 'utf8')
  const signup = source.slice(source.indexOf('// Product updates'), source.indexOf('// Injected console UI'))
  assert.match(signup, /body: JSON\.stringify\(\{ email, source: SIGNUP_SOURCE \}\)/)
  assert.doesNotMatch(source, /mailmodo/i, 'the Mailmodo form is gone')
  assert.doesNotMatch(signup, /['"]?origin['"]?\s*:\s*[A-Za-z'"`]/i, 'the API takes server-to-server calls without an Origin')
})

test('each API answer means one thing for the page', () => {
  // Sent, and already subscribed, are both a 200.
  assert.equal(signupOutcome(200, null), 'subscribed')
  assert.equal(signupOutcome(204, null), 'subscribed')
  // Only a refused address is the person's to fix.
  assert.equal(signupOutcome(400, 'invalid_email'), 'invalid_email')
  for (const [status, error] of [
    [400, 'invalid_source'],
    [403, 'forbidden'],
    [429, 'rate_limited'],
    [500, null],
    [503, 'unavailable'],
    [302, null],
    [400, null],
  ]) {
    assert.equal(signupOutcome(status, error), 'retry_later', `${status} ${error}`)
  }
})

test('reads the error code from the body, and nothing from a page that is not JSON', () => {
  assert.equal(errorCode('{"error":"invalid_email"}'), 'invalid_email')
  assert.equal(errorCode('{"ok":true}'), null)
  assert.equal(errorCode('<html>bad gateway</html>'), null)
  assert.equal(errorCode(''), null)
})

test('the address only travels over https, or plain http to this machine', () => {
  for (const url of [
    'https://iii.dev/api/subscribe',
    'http://127.0.0.1:8787/api/subscribe',
    'http://localhost:8787',
    'http://[::1]:8787/x',
  ]) {
    assert.ok(isSignupUrlAllowed(url), url)
  }
  for (const url of [
    'http://example.com/api/subscribe',
    'http://127.0.0.1.evil.example/x',
    // Userinfo, not the host: the address would go to evil.example.
    'http://localhost:80@evil.example/api/subscribe',
    'http://127.0.0.1@evil.example/',
    'http://localhost.evil.example',
    'ftp://127.0.0.1',
    'not a url',
  ]) {
    assert.equal(isSignupUrlAllowed(url), false, `${url} should be refused`)
  }
})
