import { expect, expectContains, expectEqual, type TestCase, until } from './cases.ts'

/**
 * One-shot `code-runner::run`, in both languages.
 *
 * Every case here runs `keep: false`, so nothing needs tearing down. The
 * point of running the same assertion twice is that this worker's promise is
 * ONE api over two engines — a response shape that differs by language is the
 * defect this group exists to catch.
 */
export const RUN_CASES: TestCase[] = [
  {
    name: 'all four functions are live on the bus with typed schemas',
    async run(ctx) {
      const ids = [
        'code-runner::run',
        'code-runner::teardown',
        'code-runner::register_function',
        'code-runner::inject-guidance',
      ]
      const found = await until(async () => {
        const info = await ctx.call('engine::functions::info', { function_ids: ids })
        const fns = (info?.functions ?? []).filter((f: any) => f && !f.error)
        return fns.length === ids.length ? fns : undefined
      }, 'all four code-runner functions to be live on the bus')

      const run = found.find((f: any) => f.function_id === 'code-runner::run')
      expect(
        Array.isArray(run.request_schema?.required) && run.request_schema.required.includes('code'),
        `run.request_schema.required should include "code", got ${JSON.stringify(run.request_schema?.required)}`,
      )
    },
  },

  {
    name: 'node: a one-shot run returns its value and a process-shaped response',
    async run(ctx) {
      const r = await ctx.call('code-runner::run', { lang: 'node', code: 'return 2 + 2' })
      expectEqual(r.result, 4, 'result should be the expression value')
      expectEqual(r.exit_code, 0, 'a clean run exits 0')
      expectEqual(r.success, true, 'a clean run succeeds')
      expect(r.runtime_id === undefined, `keep:false must mint no runtime_id, got ${r.runtime_id}`)
      expect(typeof r.duration_ms === 'number', 'duration_ms should be a number')
    },
  },

  {
    name: 'python: a one-shot run returns its value and a process-shaped response',
    async run(ctx) {
      const r = await ctx.call('code-runner::run', { lang: 'python', code: 'result = 2 + 2' })
      expectEqual(r.result, 4, 'result should be what the code assigned')
      expectEqual(r.exit_code, 0, 'a clean run exits 0')
      expectEqual(r.success, true, 'a clean run succeeds')
      expect(r.runtime_id === undefined, `keep:false must mint no runtime_id, got ${r.runtime_id}`)
    },
  },

  {
    name: 'both languages report stdout and stderr on the same two fields',
    async run(ctx) {
      const node = await ctx.call('code-runner::run', {
        lang: 'node',
        code: 'console.log("out-n"); console.error("err-n"); return 1',
      })
      expectContains(node.stdout, 'out-n', 'node stdout')
      expectContains(node.stderr, 'err-n', 'node stderr')

      const py = await ctx.call('code-runner::run', {
        lang: 'python',
        code: 'import sys\nprint("out-p")\nprint("err-p", file=sys.stderr)\nresult = 1',
      })
      expectContains(py.stdout, 'out-p', 'python stdout')
      expectContains(py.stderr, 'err-p', 'python stderr')
    },
  },

  {
    // Exercises the guest's ASCII-escaped envelope on the python side
    // (wrapper.py: json.dumps defaults to ensure_ascii=True) and the plain
    // UTF-8 path on the node side. Both must return the same string.
    //
    // The text is embedded in `code` rather than passed in: this wire has no
    // `payload` field — it is sandbox-code-runner's, byte for byte, and that
    // worker has none either.
    name: 'non-ascii round-trips through both engines unchanged',
    async run(ctx) {
      const text = 'olá — 日本語 ±3°'
      for (const [lang, code] of [
        ['node', `return ${JSON.stringify(text)}`],
        ['python', `result = ${JSON.stringify(text)}`],
      ] as const) {
        const r = await ctx.call('code-runner::run', { lang, code })
        expectEqual(r.result, text, `${lang} should return the string unchanged`)
      }
    },
  },

  {
    name: 'python guest code can call back into the bus through iii.trigger',
    async run(ctx) {
      const r = await ctx.call('code-runner::run', {
        lang: 'python',
        code: [
          "answer = iii.trigger({'function_id': 'code-runner::run',",
          "                      'payload': {'lang': 'node', 'code': 'return 40 + 2'}})",
          'result = answer["result"]',
        ].join('\n'),
      })
      expectEqual(r.result, 42, 'the guest should see the inner run’s result')
    },
  },
]
