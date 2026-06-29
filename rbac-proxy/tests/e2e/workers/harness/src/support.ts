/**
 * The operator-registered functions the proxy's config points at, registered
 * on the engine's **internal** listener (not through the proxy). These are
 * exactly the functions a real deployment would register from any SDK:
 *
 *   - `support::auth`        — the RBAC auth function (validates the bearer
 *                              token, returns an AuthResult with a per-session
 *                              prefix + context).
 *   - `support::middleware`  — wraps every allowed, non-`engine::` call.
 *   - `support::on-fn-reg`   — the on_function_registration hook (stamps
 *                              metadata).
 *   - `api::echo`            — an exposed target (`expose_functions: api::*`).
 *   - `secret::echo`         — a NON-exposed target (the forbidden case).
 *
 * run-tests.sh's seed wires the proxy config's `auth_function_id`,
 * `middleware_function_id`, and `on_function_registration_function_id` to these
 * ids.
 */

import { registerWorker, type IIIClient } from 'iii-sdk'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

export function registerSupport(engineUrl: string): IIIClient {
  const support = registerWorker(engineUrl)

  // RBAC auth: the proxy invokes this once per upgrade with
  // { headers, query_params, ip_address }. Reject a missing/invalid token
  // (the proxy turns a thrown auth into an upgrade rejection); otherwise grant
  // a per-tenant namespace + context.
  support.registerFunction('support::auth', async (input: Json) => {
    const authz = input?.headers?.authorization
    if (authz !== 'Bearer test-token') {
      throw new Error('UNAUTHORIZED: missing or invalid bearer token')
    }
    return {
      context: { tenant: 'itest' },
      function_registration_prefix: 'tenant1',
    }
  })

  // Exposed + forbidden echo targets.
  support.registerFunction('api::echo', async (input: Json) => input)
  support.registerFunction('secret::echo', async (input: Json) => input)

  // Middleware: receives { function_id, payload, action, context }. Proves it
  // ran (mw:true) AND invokes the real target, returning its result wrapped —
  // the spec's "middleware must invoke the target itself" contract. The
  // `function_id` is the engine-resolved target (prefixed for own fns), so a
  // direct invoke on the engine connection dispatches correctly.
  support.registerFunction('support::middleware', async (input: Json) => {
    const result = await support.trigger({
      function_id: input.function_id,
      payload: input.payload,
    })
    return { mw: true, result }
  })

  // on_function_registration hook: stamp metadata.hooked=true. Omitted result
  // fields keep their original value.
  support.registerFunction('support::on-fn-reg', async (input: Json) => {
    return { metadata: { ...(input?.metadata ?? {}), hooked: true } }
  })

  return support
}
