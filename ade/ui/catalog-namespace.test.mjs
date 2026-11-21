import assert from 'node:assert/strict'
import test from 'node:test'
import esbuild from 'esbuild'

// One function id can live in several engine namespaces (a mesh machine's
// copy of `shell::exec` lives in namespace `laptop`). The catalogue keeps
// each copy as its own row and reads and runs it where it lives.
async function loadEngine() {
  const result = await esbuild.build({
    entryPoints: ['src/catalog/engine.ts'],
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    logLevel: 'silent',
  })
  const source = result.outputFiles[0].text
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

test('two copies of one id are two rows, read and run in their namespace', async () => {
  const { listFunctions, functionInfo, invoke, functionKey } = await loadEngine()
  const calls = []
  const host = {
    iii: {
      call: async (id, payload, options) => {
        calls.push([id, payload, options])
        return id === 'engine::functions::list'
          ? {
              functions: [
                { function_id: 'shell::exec', namespace: 'laptop', worker_name: 'a' },
                { function_id: 'shell::exec', namespace: 'desk', worker_name: 'b' },
                { function_id: 'state::get', worker_name: 'state' },
              ],
            }
          : { function_id: 'shell::exec', namespace: 'laptop', worker_name: 'a' }
      },
    },
  }
  const rows = await listFunctions(host, { includeInternal: false })
  assert.deepEqual(rows.map(functionKey), ['laptop/shell::exec', 'desk/shell::exec', 'default/state::get'])
  await functionInfo(host, 'shell::exec', 'laptop')
  assert.deepEqual(calls.at(-1)[1], { function_id: 'shell::exec', namespace: 'laptop' })
  await invoke(host, 'shell::exec', {}, { namespace: 'laptop' })
  assert.equal(calls.at(-1)[2].namespace, 'laptop')
})

// A recorded call names the id only: which copy ran is known only when one
// namespace holds the id, and the copy opened is the copy Run calls.
test('a recorded call opens a copy only when its id is in one namespace', async () => {
  const { soleCopy } = await loadEngine()
  const rows = [
    { function_id: 'shell::exec', namespace: 'default' },
    { function_id: 'shell::exec', namespace: 'alpha' },
    { function_id: 'state::get', namespace: 'default' },
  ]
  assert.equal(soleCopy(rows, 'shell::exec'), null)
  assert.equal(soleCopy(rows, 'state::get'), rows[2])
  assert.equal(soleCopy(rows, 'gone::fn'), null)
})
