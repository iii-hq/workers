// node examples/catalog2/build.mjs [--watch] → examples/catalog2/dist/catalog2.js
// (the ide copies it to android/app/src/androidTest/assets/workerui/catalog2.js)
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildNativeUi } from '../../src/build.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const result = await buildNativeUi({
  entry: join(here, 'src/app.tsx'),
  outfile: join(here, 'dist/catalog2.js'),
  ns: 'catalog2',
  watch: process.argv.includes('--watch'),
})
if (result.sha256) console.log(`${result.outfile}: ${result.bytes} bytes, sha256 ${result.sha256}`)
