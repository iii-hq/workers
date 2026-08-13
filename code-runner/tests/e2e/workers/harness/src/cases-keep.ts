import { expect, expectEqual, type TestCase } from './cases.ts'

/**
 * `keep: true`, `runtime_id`, and `teardown`.
 *
 * Every case here tears down what it creates. `max_runtimes` is shared across
 * both engines, so a leak fails a LATER case with a capacity error that says
 * nothing about the real cause.
 */
export const KEEP_CASES: TestCase[] = [
  {
    name: 'node: a kept runtime persists globals and files across calls',
    async run(ctx) {
      const first = await ctx.call('code-runner::run', {
        lang: 'node',
        keep: true,
        code: 'globalThis.n = 41; await iii.files.write("f.txt", "kept"); return 1',
      })
      const id = first.runtime_id
      expect(typeof id === 'string' && id.length > 0, 'keep:true must mint a runtime_id')

      try {
        const second = await ctx.call('code-runner::run', {
          runtime_id: id,
          code: 'return [globalThis.n + 1, await iii.files.readText("f.txt")]',
        })
        expectEqual(second.result, [42, 'kept'], 'globals and files should both survive')
        expectEqual(second.runtime_id, id, 'the response should echo the runtime it ran in')
      } finally {
        await ctx.call('code-runner::teardown', { runtime_id: id })
      }
    },
  },

  {
    // The half that could not exist before park-and-loop landed: python's
    // interpreter used to be one-shot, so `keep` persisted files only.
    name: 'python: a kept runtime persists globals and files across calls',
    async run(ctx) {
      const first = await ctx.call('code-runner::run', {
        lang: 'python',
        keep: true,
        code: 'import json\nn = 41\nopen("/work/f.txt", "w").write("kept")\nresult = 1',
      })
      const id = first.runtime_id
      expect(typeof id === 'string' && id.length > 0, 'keep:true must mint a runtime_id')

      try {
        const second = await ctx.call('code-runner::run', {
          runtime_id: id,
          code: 'result = [n + 1, open("/work/f.txt").read(), json.dumps([1])]',
        })
        expectEqual(second.result, [42, 'kept', '[1]'], 'globals, files and imported modules should all survive')
      } finally {
        await ctx.call('code-runner::teardown', { runtime_id: id })
      }
    },
  },

  {
    name: 'a torn-down runtime_id is gone, in both languages',
    async run(ctx) {
      for (const lang of ['node', 'python'] as const) {
        const started = await ctx.call('code-runner::run', {
          lang,
          keep: true,
          code: lang === 'node' ? 'return 1' : 'result = 1',
        })
        const torn = await ctx.call('code-runner::teardown', { runtime_id: started.runtime_id })
        expectEqual(torn.torn_down, true, `${lang}: teardown should report success`)

        await ctx.expectError(
          () => ctx.call('code-runner::run', { runtime_id: started.runtime_id, code: 'return 1' }),
          'code-runner::runtime_not_found',
        )
      }
    },
  },

  {
    // Routing must not depend on reading the id — it is documented as a
    // capability-secret — so the worker keeps an ownership map instead.
    name: 'a runtime_id routes to its own engine without being told the language',
    async run(ctx) {
      const py = await ctx.call('code-runner::run', {
        lang: 'python',
        keep: true,
        code: 'marker = "python"\nresult = 1',
      })
      try {
        // No `lang` on this call at all: the id alone has to route it.
        const back = await ctx.call('code-runner::run', {
          runtime_id: py.runtime_id,
          code: 'result = marker',
        })
        expectEqual(back.result, 'python', 'the id should have routed to the python engine')

        // And asking for the wrong one is refused rather than silently obeyed.
        await ctx.expectError(
          () => ctx.call('code-runner::run', { runtime_id: py.runtime_id, lang: 'node', code: 'return 1' }),
          'code-runner::invalid_request',
        )
      } finally {
        await ctx.call('code-runner::teardown', { runtime_id: py.runtime_id })
      }
    },
  },
]
