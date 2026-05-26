/**
 * Defense-in-depth allow/deny policy for the per-conversation
 * auto-accept toggle. The server-side approval gate is still in the
 * chain regardless; this policy is a client-side guard that refuses to
 * silently click "allow" on calls whose function id matches an obvious
 * high-risk pattern (state mutation, deletion, process spawning,
 * external egress, recursive trigger).
 *
 * Auto-accept exists because tight agent loops are painful when every
 * read needs a click. The threat model the policy addresses: a
 * prompt-injection payload smuggled through tool output, web content,
 * or a malicious local model server flips the model into emitting a
 * destructive `agent_trigger` that auto-accept would otherwise resolve
 * without a human in the loop.
 *
 * The default deny list is conservative — when in doubt, surface the
 * approval card to the user. Operators who want to widen or narrow
 * scope can swap in a custom policy via `isAutoAcceptable`'s second
 * parameter.
 */

export interface AutoAcceptPolicy {
  /** Regex patterns that ALWAYS require a human click. */
  readonly denyPatterns: ReadonlyArray<RegExp>
  /** Explicit function ids that ALWAYS require a human click. */
  readonly denyExact: ReadonlySet<string>
  /** Regex patterns that are ALWAYS safe to auto-accept, overriding deny rules. */
  readonly allowPatterns: ReadonlyArray<RegExp>
  /** Explicit function ids that are ALWAYS safe to auto-accept, overriding deny rules. */
  readonly allowExact: ReadonlySet<string>
}

/**
 * Worker/function-id keyword fragments that always require a human
 * click. Anchored at id boundaries: `foo::write_thing` and
 * `write::file` both match `write`, but `metadata::is_writeable` does
 * not.
 *
 * Grouped for readability; the policy treats the union as the deny
 * surface.
 */
const DENY_KEYWORDS: ReadonlyArray<string> = [
  // state mutation
  'write',
  'create',
  'update',
  'set',
  'put',
  'patch',
  'mutate',
  // deletion / removal
  'delete',
  'remove',
  'destroy',
  'drop',
  'purge',
  'rm',
  // process / system-level invocation. Matched as boundary tokens
  // (last segment, or a `::`-delimited segment), so `shell::fs::read`
  // does NOT trigger on the `shell::` worker namespace prefix while
  // `shell::exec` / `proc::run` / `vm::eval` correctly deny.
  'exec',
  'run',
  'eval',
  'spawn',
  'run_command',
  'kill',
  // external egress / network side-effects
  'send',
  'post',
  'publish',
  'email',
  'notify',
  'webhook',
  // deploy / install / push
  'deploy',
  'install',
  'uninstall',
  'push',
  'pull_request',
  // recursive dispatch — would let auto-accept escalate transitively
  'trigger',
  'dispatch',
  // credentials / secrets
  'credential',
  'secret',
  'token',
  'key',
  'auth',
]

function boundaryDenyRegex(keyword: string): RegExp {
  return new RegExp(`(?:^|::)${keyword}(?:$|_|::)`, 'i')
}

export const DEFAULT_DENY_PATTERNS: ReadonlyArray<RegExp> =
  DENY_KEYWORDS.map(boundaryDenyRegex)

export const DEFAULT_DENY_EXACT: ReadonlySet<string> = new Set([
  'agent::trigger',
  'iii::durable::publish',
])

/** Regex patterns that whitelist entire namespaces (checked before deny rules). */
export const DEFAULT_ALLOW_PATTERNS: ReadonlyArray<RegExp> = [/^sandbox::/]

/** Explicit function ids that are safe to auto-accept even if their name contains a deny keyword. */
export const DEFAULT_ALLOW_EXACT: ReadonlySet<string> = new Set([
  'sandbox::create',
])

export const DEFAULT_POLICY: AutoAcceptPolicy = {
  denyPatterns: DEFAULT_DENY_PATTERNS,
  denyExact: DEFAULT_DENY_EXACT,
  allowPatterns: DEFAULT_ALLOW_PATTERNS,
  allowExact: DEFAULT_ALLOW_EXACT,
}

/**
 * Returns `true` if `functionId` is safe to auto-accept under `policy`.
 * Treats undefined/empty function ids as unsafe (fail-closed).
 *
 * Precedence: allowPatterns > allowExact > denyExact > denyPatterns.
 */
export function isAutoAcceptable(
  functionId: string | undefined,
  policy: AutoAcceptPolicy = DEFAULT_POLICY,
): boolean {
  if (typeof functionId !== 'string' || functionId.length === 0) return false
  for (const re of policy.allowPatterns) {
    if (re.test(functionId)) return true
  }
  if (policy.allowExact.has(functionId)) return true
  if (policy.denyExact.has(functionId)) return false
  for (const re of policy.denyPatterns) {
    if (re.test(functionId)) return false
  }
  return true
}
