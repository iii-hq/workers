import { execFileSync } from 'node:child_process'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { expect, test } from './context-tracking-stack'

test.skip(
  !process.env.CONTEXT_ENGINE_BIN ||
    !process.env.CONTEXT_SESSION_BIN ||
    !process.env.CONTEXT_CONSOLE_BIN,
  'Interface smoke requires real engine, session-manager and Console binaries',
)

test('Context functions publish typed schemas accepted by the CI interface validator', async ({
  stack,
}, info) => {
  const functions = []
  for (const function_id of [
    'console::context::list',
    'console::context::save',
    'console::context::remove',
    'console::context::inject-guidance',
  ]) {
    const detail = await stack.call('engine::functions::info', { function_id })
    functions.push({
      name: function_id,
      request_schema: detail.request_schema,
      response_schema: detail.response_schema,
    })
  }
  const file = info.outputPath('context-interface.json')
  await writeFile(file, JSON.stringify({ functions, triggers: [] }))
  const script = path.resolve(
    import.meta.dirname,
    '../../../.github/scripts/collect_worker_interface.py',
  )
  execFileSync('python3', [
    script,
    '--assert-non-empty',
    '--assert-typed-schemas',
    '--assert-file',
    file,
  ])
  const session_id = await stack.create('Typed guidance')
  expect(
    await stack.call('console::context::inject-guidance', {
      session_id,
      generate: { system_prompt: 'BASE', tools: [] },
    }),
  ).toEqual({ mutations: {} })
  const result = await stack.call('console::context::inject-guidance', {
    session_id,
    generate: {
      system_prompt: 'BASE',
      tools: [{ name: 'console::context::save' }],
    },
  })
  expect(result.mutations.system_prompt).toContain('console::context::save')
  await info.attach('context-interface', {
    path: file,
    contentType: 'application/json',
  })
})
