/**
 * The code-runner function-trigger renderers — one module per rendered op:
 *
 *   code-runner::run               → ./run
 *   code-runner::register_function → ./register-function
 *   code-runner::teardown          → ./teardown
 *
 * Registered through `host.functionTriggers`, so they dispatch BEFORE the
 * console's first-party families and override how those calls render in chat
 * and in the traces span tab.
 *
 * The console asks EVERY registered renderer on every message (`isMatch` is
 * only used to pick a `FunctionIdLabel`), so each `tryRender*` gates on its own
 * function id and returns null to fall through.
 *
 * These cards do NOT fall through on error outputs: code-runner's error
 * messages quote the `runtime_id` by design (`unknown runtime_id {id}` —
 * error.rs), and a runtime id is a capability, so the console's default error
 * view would print it verbatim. Each card renders its own `ErrorCard` with the
 * message routed through `redactRuntimeIds`.
 *
 * `code-runner::inject-guidance` is deliberately NOT rendered: it is a
 * harness-internal `pre_generate` hook that appends usage guidance to an
 * agent's system prompt, not a call anyone makes on purpose. It keeps the
 * console's default card.
 */

import type { FunctionTriggerRenderer, Host } from '@iii-dev/console-ui'
import { redactRuntimeIdsDeep } from '../lib/shared'
import { createRegisterFunctionRenderer } from './register-function'
import { createRunRenderer } from './run'
import { createTeardownRenderer } from './teardown'

/**
 * `redactRaw` is attached here rather than card by card: the console's
 * function-trigger card mounts a `raw json` tab (and a copy button) that
 * renders `input`/`output` verbatim whatever the card does, and EVERY
 * code-runner payload can carry a `runtime_id` — in a field, in a line of
 * program output, in a returned completion value, in one of the error messages
 * that quote it by design (error.rs). It is a property of the worker's
 * payloads, not of any one op, so a renderer cannot opt out by omission.
 */
export function createCodeRunnerRenderers(host: Host): FunctionTriggerRenderer[] {
  return [createRunRenderer(host), createRegisterFunctionRenderer(host), createTeardownRenderer(host)].map(
    (renderer) => ({ ...renderer, redactRaw: redactRuntimeIdsDeep }),
  )
}
