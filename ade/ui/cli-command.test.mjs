import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import test from 'node:test'
import esbuild from 'esbuild'

// The Functions page copies a call as an `iii call` line for a terminal.
// `iii call` reads each `key=value` value as JSON first and keeps it as a
// string only when it does not parse (`id=42` is the integer 42), and the
// line runs in a POSIX shell. So a copied command must keep every value's
// type and must never let the shell expand, split or redirect a value.
async function loadCliCommand() {
  const result = await esbuild.build({
    entryPoints: ['src/catalog/cli-command.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

const { asCliCommand, cliArgs, shellWord } = await loadCliCommand()

/**
 * Run `command` in /bin/sh with `iii` replaced by a stand-in that prints
 * its arguments, then read them back the way `iii call` does: split each
 * `key=value` at the first `=`, JSON first, plain string otherwise.
 */
function throughShell(command) {
  const script = `iii() { for a in "$@"; do printf '%s\\0' "$a"; done; }\n${command}`
  const argv = execFileSync('/bin/sh', ['-c', script], { encoding: 'utf8' }).split('\0').slice(0, -1)
  assert.equal(argv[0], 'call')
  const payload = {}
  for (const token of argv.slice(2)) {
    const at = token.indexOf('=')
    const value = token.slice(at + 1)
    try {
      payload[token.slice(0, at)] = JSON.parse(value)
    } catch {
      payload[token.slice(0, at)] = value
    }
  }
  return { functionId: argv[1], payload }
}

test('a string the CLI would parse as another type goes as JSON', () => {
  assert.equal(asCliCommand('engine::functions::info', { function_id: '42' }), `iii call engine::functions::info function_id='"42"'`)
  for (const text of ['42', 'true', 'null', '[1]', '{"a":1}', ' 7 ']) {
    assert.deepEqual(throughShell(asCliCommand('a::b', { v: text })).payload, { v: text }, text)
  }
})

test('plain strings, numbers, booleans and null read as typed', () => {
  assert.equal(
    asCliCommand('a::b', { id: 'ade::status', n: 42, on: true, none: null }),
    'iii call a::b id=ade::status n=42 on=true none=null',
  )
})

test('an empty string keeps its quotes', () => {
  assert.equal(asCliCommand('ade::compose::search', { query: '' }), `iii call ade::compose::search query=''`)
  assert.deepEqual(throughShell(asCliCommand('a::b', { query: '' })).payload, { query: '' })
})

test('shell metacharacters are quoted, never expanded or split', () => {
  const hostile = ['$(id)', '`id`', 'a;b', 'a&b', 'a|b', '*.json', 'x>y', '~/p', '#tag', "it's", 'two words', 'a\\b', '$HOME', '!!']
  for (const text of hostile) {
    const command = asCliCommand('a::b', { v: text })
    assert.deepEqual(throughShell(command).payload, { v: text }, command)
  }
})

test('nested values go as JSON and come back equal', () => {
  const body = { obj: { a: 1, b: ["x y", null] }, list: [1, 'two'], flag: false }
  assert.deepEqual(throughShell(asCliCommand('a::b', body)).payload, body)
})

test('a key the CLI cannot read bare quotes the whole token', () => {
  const [arg] = cliArgs({ 'a b': 1 })
  assert.equal(arg.key, '')
  assert.equal(arg.value, `'a b=1'`)
  assert.deepEqual(throughShell(asCliCommand('a::b', { 'a b': 1, 'k;x': 's' })).payload, { 'a b': 1, 'k;x': 's' })
})

test('the function id is a shell word too', () => {
  assert.equal(shellWord('ade::status'), 'ade::status')
  assert.equal(throughShell(asCliCommand('weird id;x', {})).functionId, 'weird id;x')
})

test('a body that is not an object has no arguments', () => {
  assert.equal(asCliCommand('a::b', null), 'iii call a::b')
  assert.equal(asCliCommand('a::b', [1, 2]), 'iii call a::b')
})
