// The worker UI bench bundle (ide: projects/worker-native-ui/medicoes.md, design §11).
//
//   node bench/build.mjs [--out DIR]
//
// Builds bench/src/bench.tsx with buildNativeUi (namespace `bench`) into bench/dist/bench.js, checks every page
// in the test harness (QuickJS in Wasm, a fake host), and with --out also copies it there: the ide repo keeps it
// in android/app/src/androidTest/assets/workerui-bench/ (ide: projects/worker-native-ui/bench/build-workerui-bench.sh).

import { readFileSync } from 'node:fs'
import { copyFile, mkdir } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildNativeUi } from '../src/build.mjs'
import { createTestHost } from '../src/testing/index.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const outIndex = process.argv.indexOf('--out')
const out = outIndex > 0 ? resolve(process.argv[outIndex + 1] ?? '') : null

const result = await buildNativeUi({
  entry: join(here, 'src/bench.tsx'),
  outfile: join(here, 'dist/bench.js'),
  ns: 'bench',
})
for (const w of result.warnings) console.warn(w)
const bundle = readFileSync(result.outfile, 'utf8')

const fail = (why) => {
  console.error(`bench: ${why}`)
  process.exit(1)
}
const reports = []
const host = await createTestHost({
  bundle,
  ns: 'bench',
  calls: {
    'bench::report': (payload) => {
      reports.push(payload)
      return null
    },
    'bench::echo': (payload) => payload,
  },
})
try {
  await host.mount('root')
  if (host.prop('hello', 'text') !== 'Worker UI bench') fail('root shows no text')

  const list = await host.mount('list', { rows: 1000, field: true })
  if (host.find('list').items.length !== 1000) fail('list has no 1000 items')
  await host.itemPress('list', 7)
  if (host.prop('taps', 'text') !== 'taps 1') fail('a press did not count')
  if (host.find('list').items[7].count !== '1') fail('a press did not reach its row')
  await host.type('field', ['a', 'ab'])
  await host.unmount(list)

  await host.mount('prop', { hz: 60, seconds: 0.2 })
  for (let i = 0; i < 20; i++) await host.tick(17)
  if (!reports.some((r) => r.kind === 'prop' && r.ticks >= 12)) fail('prop did not tick')

  const text = await host.mount('text', { busyMs: 0, upper: true })
  await host.type('field', ['a', 'ab'])
  if (host.prop('echo', 'text') !== 'AB') fail('text did not echo')
  await host.unmount(text)

  await host.mount('rtt', { fn: 'bench::echo', count: 3, gapMs: 1 })
  for (let i = 0; i < 10; i++) await host.tick(5)
  if (!JSON.parse(host.prop('rtt-result', 'text') || '{}').rttMs) fail('rtt did not report')

  await host.mount('remote', { session: 's1' })
  if (host.prop('screen', 'session') !== 's1') fail('remote has no RemoteScreen on s1')

  await host.mount('hog', { mb: 2 })
  if (host.prop('hog', 'text') !== 'holding 2 MiB') fail('hog holds nothing')
  if (host.errors.length || host.exceptions.length || host.violations.length) {
    fail(JSON.stringify({ errors: host.errors, exceptions: host.exceptions, violations: host.violations }))
  }
} finally {
  host.dispose()
}

console.log(`${result.outfile}: ${result.bytes} bytes, sha256 ${result.sha256}`)
if (out) {
  await mkdir(out, { recursive: true })
  await copyFile(result.outfile, join(out, 'bench.js'))
  console.log(`copied to ${join(out, 'bench.js')}`)
}
