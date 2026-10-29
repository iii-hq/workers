// node build.mjs [--watch] → dist/native.js, embedded by src/native_ui.rs
// (build.rs runs this when the sources are newer than the bundle).
import { buildNativeUi } from '@iii-dev/native-ui/build'

const result = await buildNativeUi({
  entry: 'src/app.tsx',
  outfile: 'dist/native.js',
  ns: 'browser',
  watch: process.argv.includes('--watch'),
})
if (result.sha256) console.log(`dist/native.js: ${result.bytes} bytes, sha256 ${result.sha256}`)
