/**
 * The operator-registered functions the proxy's config points at, registered
 * on the engine's **internal** listener (not through the proxy). These are
 * exactly the functions a real deployment would register from any SDK:
 *
 *   - `support::auth`        — the RBAC auth function (validates the bearer
 *                              token, returns an AuthResult with the token's
 *                              tenant prefix + context: `TENANT1_TOKEN` gets
 *                              `tenant1`, `TENANT2_TOKEN` gets `tenant2`).
 *   - `support::middleware`  — wraps every allowed, non-`engine::` call and
 *                              records its `function_id` in `middlewareCalls`.
 *   - `support::on-fn-reg`   — the on_function_registration hook (stamps
 *                              metadata).
 *   - `support::on-observer-reg` — the on_observer_registration hook (denies
 *                              an observer whose args say `deny: true`).
 *   - `api::echo`            — an exposed target (`expose_functions: api::*`).
 *   - `secret::echo`         — a NON-exposed target (the forbidden case).
 *   - `api::sink`            — an exposed observer target.
 *
 * run-tests.sh's seed wires the proxy config's `auth_function_id`,
 * `middleware_function_id`, `on_function_registration_function_id` and
 * `on_observer_registration_function_id` to these ids.
 *
 * The auth, middleware and hook functions receive bearer tokens, payloads and
 * session context, so they are registered `observable: false`: nobody may
 * observe them.
 */

import { registerWorker, type IIIClient } from 'iii-sdk'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** The `function_id` of every call the middleware received, in order. */
export const middlewareCalls: string[] = []

/** The bearer token of a `tenant1` session (the harness's `down` among them). */
export const TENANT1_TOKEN = 'Bearer test-token'
/** The bearer token of a `tenant2` session. */
export const TENANT2_TOKEN = 'Bearer test-token-tenant2'

/** The AuthResult each accepted token gets. */
const TENANTS: Record<string, { context: Json; function_registration_prefix: string }> = {
  [TENANT1_TOKEN]: { context: { tenant: 'itest' }, function_registration_prefix: 'tenant1' },
  [TENANT2_TOKEN]: { context: { tenant: 'itest2' }, function_registration_prefix: 'tenant2' },
}

export function registerSupport(engineUrl: string): IIIClient {
  const support = registerWorker(engineUrl, {
    workerName: 'rbac-e2e-support',
    otel: { enabled: false },
  })

  // RBAC auth: the proxy invokes this once per upgrade with
  // { headers, query_params, ip_address }. Reject a missing/invalid token
  // (the proxy turns a thrown auth into an upgrade rejection); otherwise grant
  // the token's tenant prefix + context.
  support.registerFunction(
    'support::auth',
    async (input: Json) => {
      const tenant = TENANTS[input?.headers?.authorization]
      if (!tenant) {
        throw new Error('UNAUTHORIZED: missing or invalid bearer token')
      }
      return tenant
    },
    { observable: false },
  )

  // Exposed + forbidden echo targets.
  support.registerFunction('api::echo', async (input: Json) => input)
  support.registerFunction('secret::echo', async (input: Json) => input)
  // An exposed observer target.
  support.registerFunction('api::sink', async () => null)

  // Middleware: receives { function_id, payload, action, context }. Proves it
  // ran (mw:true) AND invokes the real target, returning its result wrapped —
  // the spec's "middleware must invoke the target itself" contract. The
  // `function_id` is the engine-resolved target (prefixed for own fns), so a
  // direct invoke on the engine connection dispatches correctly. Each call is
  // recorded in `middlewareCalls`.
  support.registerFunction(
    'support::middleware',
    async (input: Json) => {
      middlewareCalls.push(input.function_id)
      const result = await support.call({
        function_id: input.function_id,
        payload: input.payload,
      })
      return { mw: true, result }
    },
    { observable: false },
  )

  // on_function_registration hook: stamp metadata.hooked=true. Omitted result
  // fields keep their original value.
  support.registerFunction(
    'support::on-fn-reg',
    async (input: Json) => {
      return { metadata: { ...(input?.metadata ?? {}), hooked: true } }
    },
    { observable: false },
  )

  // on_observer_registration hook: an object answer lets the observer
  // through unchanged; a throw denies it (FORBIDDEN).
  support.registerFunction(
    'support::on-observer-reg',
    async (input: Json) => {
      if (input?.args?.deny === true) {
        throw new Error('DENIED: the observer asked to be denied')
      }
      return {}
    },
    { observable: false },
  )

  return support
}
