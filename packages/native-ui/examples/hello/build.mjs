// node examples/hello/build.mjs [--watch] → examples/hello/dist/native.js
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildNativeUi } from '../../src/build.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const result = await buildNativeUi({
  entry: join(here, 'src/app.tsx'),
  outfile: join(here, 'dist/native.js'),
  ns: 'hello',
  watch: process.argv.includes('--watch'),
})
if (result.sha256) console.log(`${result.outfile}: ${result.bytes} bytes, sha256 ${result.sha256}`)
