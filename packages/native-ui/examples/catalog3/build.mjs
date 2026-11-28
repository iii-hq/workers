// node examples/catalog3/build.mjs [--watch] → examples/catalog3/dist/catalog3.js
// (the ide copies it to android/app/src/androidTest/assets/workerui/catalog3.js
// and to the Apple UI tests' resources)
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildNativeUi } from '../../src/build.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const result = await buildNativeUi({
  entry: join(here, 'src/app.tsx'),
  outfile: join(here, 'dist/catalog3.js'),
  ns: 'catalog3',
  watch: process.argv.includes('--watch'),
})
if (result.sha256) console.log(`${result.outfile}: ${result.bytes} bytes, sha256 ${result.sha256}`)
