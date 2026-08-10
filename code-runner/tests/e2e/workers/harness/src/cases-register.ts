import { expect, expectContains, expectEqual, type TestCase, until } from './cases.ts'

/**
 * `code-runner::register_function` — a handler published on the real bus and
 * invoked through it.
 *
 * This is the group that most needs to be end-to-end. A unit test can prove
 * the router calls `Engine::register`; only a live engine proves the id
 * reaches the catalog, that a caller can trigger it, and that its answer comes
 * back over the wire. Every case tears its namespace down.
 */
export const REGISTER_CASES: TestCase[] = [
  {
    name: 'node: a registered handler is callable through the bus',
    async run(ctx) {
      const r = await ctx.call('code-runner::register_function', {
        function_id: 'e2e-node::greet',
        lang: 'node',
        description: 'e2e node handler',
        // Node's source DEFINES `handler`, exactly like python's — the
        // worker wraps it and publishes on its behalf. Calling
        // `iii.registerFunction` here instead would register a second time
        // under the same id, which is a registration failure, not a shortcut.
        source: 'function handler(p) { return `hello ${p.who}` }',
      })
      expectEqual(r.registered, true, 'registration should report success')

      try {
        await until(
          () => ctx.call('e2e-node::greet', { who: 'world' }).catch(() => undefined),
          'e2e-node::greet to be live on the bus',
        )
        const answer = await ctx.call('e2e-node::greet', { who: 'world' })
        expectEqual(answer, 'hello world', 'the handler should answer through the bus')
      } finally {
        await ctx.call('code-runner::teardown', { namespace: 'e2e-node' })
      }
    },
  },

  {
    // The capability this branch added. Python's guest has no
    // `iii.registerFunction` — `python.wasm` exports `_start` and nothing
    // else — so the host publishes on its behalf and dispatches each
    // invocation as one turn on the namespace's pinned interpreter.
    name: 'python: a registered handler is callable through the bus',
    async run(ctx) {
      const r = await ctx.call('code-runner::register_function', {
        function_id: 'e2e-py::greet',
        lang: 'python',
        description: 'e2e python handler',
        source: 'def handler(payload):\n    return "hello " + payload["who"]',
      })
      expectEqual(r.registered, true, 'registration should report success')

      try {
        await until(
          () => ctx.call('e2e-py::greet', { who: 'world' }).catch(() => undefined),
          'e2e-py::greet to be live on the bus',
        )
        const answer = await ctx.call('e2e-py::greet', { who: 'world' })
        expectEqual(answer, 'hello world', 'the handler should answer through the bus')
      } finally {
        await ctx.call('code-runner::teardown', { namespace: 'e2e-py' })
      }
    },
  },

  {
    // The reason python `register_function` could not exist before
    // park-and-loop: a handler needs an interpreter that outlives the call.
    name: 'python: a handler keeps the state its registration built',
    async run(ctx) {
      await ctx.call('code-runner::register_function', {
        function_id: 'e2e-state::next',
        lang: 'python',
        source: [
          'import itertools',
          '_seq = itertools.count(1)',
          'def handler(payload):',
          '    return next(_seq)',
        ].join('\n'),
      })

      try {
        await until(
          () => ctx.call('e2e-state::next', {}).catch(() => undefined),
          'e2e-state::next to be live on the bus',
        )
        // The first call may have been consumed by the `until` probe above,
        // so assert the SEQUENCE rather than absolute values.
        const a = await ctx.call('e2e-state::next', {})
        const b = await ctx.call('e2e-state::next', {})
        expectEqual(b, a + 1, 'the counter should advance across invocations')
      } finally {
        await ctx.call('code-runner::teardown', { namespace: 'e2e-state' })
      }
    },
  },

  {
    name: 'python: source that defines no handler fails the registration',
    async run(ctx) {
      const err = await ctx.expectError(
        () =>
          ctx.call('code-runner::register_function', {
            function_id: 'e2e-bad::nope',
            lang: 'python',
            source: 'x = 1  # no handler here',
          }),
        'code-runner::invalid_request',
      )
      expectContains(err.message ?? '', 'handler(payload)', 'the refusal should say what to write')

      // Nothing was published, so the namespace does not exist to tear down.
      await ctx.expectError(
        () => ctx.call('code-runner::teardown', { namespace: 'e2e-bad' }),
        'code-runner::runtime_not_found',
      )
    },
  },

  {
    // Ids are claimed in ONE registry across both engines. Two registries
    // would each believe they owned the id, both would reach the SDK's
    // register_function, and the second would abort the process on its
    // duplicate-id panic — so this case doubles as a liveness check: if the
    // worker died, every later case fails too.
    name: 'a function id cannot be claimed by both engines',
    async run(ctx) {
      await ctx.call('code-runner::register_function', {
        function_id: 'e2e-clash::fn',
        lang: 'python',
        source: 'def handler(payload):\n    return 1',
      })

      try {
        await ctx.expectError(
          () =>
            ctx.call('code-runner::register_function', {
              function_id: 'e2e-clash::fn',
              lang: 'node',
              source: 'function handler() { return 2 }',
            }),
          'code-runner::invalid_request',
        )

        // The worker is still serving, which is the half a duplicate-id panic
        // would have taken out.
        const alive = await ctx.call('code-runner::run', { lang: 'node', code: 'return 1' })
        expectEqual(alive.result, 1, 'the worker should still be serving runs')
      } finally {
        await ctx.call('code-runner::teardown', { namespace: 'e2e-clash' })
      }
    },
  },

  {
    name: 'teardown by namespace unregisters the functions it reports',
    async run(ctx) {
      for (const id of ['e2e-down::a', 'e2e-down::b']) {
        await ctx.call('code-runner::register_function', {
          function_id: id,
          lang: 'python',
          source: `def handler(payload):\n    return ${JSON.stringify(id)}`,
        })
      }
      await until(() => ctx.call('e2e-down::a', {}).catch(() => undefined), 'e2e-down::a to be live on the bus')

      const torn = await ctx.call('code-runner::teardown', { namespace: 'e2e-down' })
      expectEqual(torn.torn_down, true, 'teardown should report success')
      expectEqual(torn.namespace, 'e2e-down::', 'the namespace should come back canonical')
      expectEqual(
        [...(torn.unregistered ?? [])].sort(),
        ['e2e-down::a', 'e2e-down::b'],
        'both functions should be reported unregistered',
      )

      // Gone from the bus, and the id is free for a fresh registration.
      await until(
        async () => ((await ctx.call('e2e-down::a', {}).catch(() => 'gone')) === 'gone' ? true : undefined),
        'e2e-down::a to disappear from the bus',
      )
      const again = await ctx.call('code-runner::register_function', {
        function_id: 'e2e-down::a',
        lang: 'python',
        source: 'def handler(payload):\n    return "reused"',
      })
      expect(again.registered === true, 'teardown must release the ids it unregistered')
      await ctx.call('code-runner::teardown', { namespace: 'e2e-down' })
    },
  },
]
