// Writes fixtures/<board>/state.json from src/fixtures/index.ts: `node fixtures/sync.mjs`, then `biome format --write fixtures`.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import esbuild from 'esbuild'

const here = dirname(fileURLToPath(import.meta.url))
const out = await esbuild.build({
  entryPoints: [join(here, '../src/fixtures/index.ts')],
  bundle: true,
  format: 'esm',
  write: false,
})
const { boards } = await import(`data:text/javascript;base64,${Buffer.from(out.outputFiles[0].text).toString('base64')}`)
for (const [id, board] of Object.entries(boards)) {
  mkdirSync(join(here, id), { recursive: true })
  writeFileSync(join(here, id, 'state.json'), `${JSON.stringify({ version: 1, ...board.status }, null, 2)}\n`)
}
console.log(`wrote ${Object.keys(boards).length} fixtures`)
