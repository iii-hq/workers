import { type TestCase, expectEqual, expectContains } from './cases.ts'

/**
 * The taxonomy, over the real wire.
 *
 * The load-bearing distinction this worker inherits from
 * `sandbox-code-runner`: **a failing script is a RESPONSE, not an error.** A
 * tenant exception comes back as a resolved call with `exit_code: 1` and the
 * traceback in `stderr`; errors are reserved for infrastructure. Getting this
 * backwards is the single most likely way to break a caller written against
 * the sibling worker, so both halves are asserted in both languages.
 */
export const ERROR_CASES: TestCase[] = [
  {
    name: 'node: a thrown error is a response with exit_code 1, not a bus error',
    async run(ctx) {
      const r = await ctx.call('code-runner::run', {
        lang: 'node',
        code: 'throw new Error("boom-n")',
      })
      expectEqual(r.exit_code, 1, 'a thrown script exits 1')
      expectEqual(r.success, false, 'a thrown script does not succeed')
      expectContains(r.stderr, 'boom-n', 'the message belongs in stderr')
    },
  },

  {
    name: 'python: a raised exception is a response with exit_code 1, not a bus error',
    async run(ctx) {
      const r = await ctx.call('code-runner::run', {
        lang: 'python',
        code: 'raise ValueError("boom-p")',
      })
      expectEqual(r.exit_code, 1, 'a raising script exits 1')
      expectEqual(r.success, false, 'a raising script does not succeed')
      expectContains(r.stderr, 'boom-p', 'the message belongs in stderr')
    },
  },

  {
    name: 'python: a syntax error is a response too, naming the line',
    async run(ctx) {
      const r = await ctx.call('code-runner::run', { lang: 'python', code: 'def (:' })
      expectEqual(r.exit_code, 1, 'a syntax error exits 1')
      expectContains(r.stderr, 'line', 'the message should name the line')
    },
  },

  {
    name: 'a script that keeps printing still returns, and its logs are capped',
    async run(ctx) {
      const r = await ctx.call('code-runner::run', {
        lang: 'python',
        code: 'for i in range(200000):\n    print("x" * 40)\nresult = "done"',
        timeout_ms: 25000,
      })
      // Either it finished or it hit its budget; what must NOT happen is an
      // unbounded response. 1 MiB is the cap each stream is held to.
      expectEqual(
        r.stdout.length <= 2 * 1024 * 1024,
        true,
        `stdout should be capped, got ${r.stdout.length} bytes`,
      )
    },
  },

  {
    name: 'an infinite loop is killed at its own timeout, and the worker survives',
    async run(ctx) {
      for (const [lang, code] of [
        ['node', 'while (true) {}'],
        ['python', 'while True:\n    pass'],
      ] as const) {
        await ctx.expectError(
          () => ctx.call('code-runner::run', { lang, code, timeout_ms: 1500 }),
          'code-runner::timeout',
        )
        // Still serving, which is the whole point of the kill.
        const after = await ctx.call('code-runner::run', {
          lang,
          code: lang === 'node' ? 'return 1' : 'result = 1',
        })
        expectEqual(after.result, 1, `${lang} should still be serving after a timeout`)
      }
    },
  },

  {
    name: 'a run with no lang and no runtime_id is refused, saying which to pass',
    async run(ctx) {
      const err = await ctx.expectError(
        () => ctx.call('code-runner::run', { code: 'return 1' }),
        'code-runner::invalid_request',
      )
      expectContains(err.message ?? '', 'lang', 'the refusal should name the missing field')
    },
  },

  {
    name: 'an unknown runtime_id is not found, not a generic engine error',
    async run(ctx) {
      await ctx.expectError(
        () => ctx.call('code-runner::run', { runtime_id: 'rt-does-not-exist', code: 'return 1' }),
        'code-runner::runtime_not_found',
      )
    },
  },

  {
    name: 'teardown refuses both runtime_id and namespace together, and neither',
    async run(ctx) {
      await ctx.expectError(
        () => ctx.call('code-runner::teardown', { runtime_id: 'rt-x', namespace: 'ns' }),
        'code-runner::invalid_request',
      )
      await ctx.expectError(
        () => ctx.call('code-runner::teardown', {}),
        'code-runner::invalid_request',
      )
    },
  },
]
