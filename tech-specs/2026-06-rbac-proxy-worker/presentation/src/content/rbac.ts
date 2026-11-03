/* rbac — the contract, for the rbac-contract deep dive (rbac.md). */

export interface FieldRow {
  name: string
  type: string
  desc: string
}

export const AUTH_INPUT_FIELDS: FieldRow[] = [
  { name: 'headers', type: 'Record<string, string>', desc: 'http headers from the upgrade request, e.g. authorization.' },
  { name: 'query_params', type: 'Record<string, string[]>', desc: 'query params; repeated keys preserved.' },
  { name: 'ip_address', type: 'string', desc: 'connecting client ip as the proxy sees it.' },
]

export const AUTH_RESULT_FIELDS: FieldRow[] = [
  { name: 'allowed_functions', type: 'string[] = []', desc: 'allow beyond expose_functions.' },
  { name: 'forbidden_functions', type: 'string[] = []', desc: 'deny even if exposed. highest precedence.' },
  { name: 'allow_function_registration', type: 'boolean = true', desc: 'may register new functions, event sources included.' },
  { name: 'function_registration_prefix', type: 'string | omitted', desc: 'private namespace prefix.' },
  { name: 'context', type: 'object = {}', desc: 'forwarded to middleware + hooks.' },
]

export const MIDDLEWARE_FIELDS: FieldRow[] = [
  { name: 'function_id', type: 'string', desc: 'the function the worker wants to invoke.' },
  { name: 'payload', type: 'object', desc: 'the payload the worker sent.' },
  { name: 'action', type: 'CallAction | omitted', desc: 'enqueue / void, if any.' },
  { name: 'context', type: 'object', desc: 'auth result context for this session.' },
]

export interface HookRow {
  hook: string
  firesOn: string
  result: string
}

export const HOOK_ROWS: HookRow[] = [
  {
    hook: 'on_function_registration',
    firesOn: 'RegisterFunction',
    result: '{ function_id?, description?, metadata?, private?, observable?, args_format? }',
  },
  {
    hook: 'on_observer_registration',
    firesOn: 'Observe',
    result: '{ function_id?, target_function_id?, args?, filters?, metadata? }',
  },
]

export const RESOLUTION_RULES = [
  { n: '1', text: 'function_id in forbidden_functions', verdict: 'deny' },
  { n: '2', text: 'function_id in allowed_functions', verdict: 'allow' },
  { n: '3', text: 'function_id in the infrastructure carve-out', verdict: 'allow' },
  { n: '4', text: 'any expose_functions filter matches', verdict: 'allow' },
  { n: '5', text: 'otherwise', verdict: 'deny' },
] as const

export interface Divergence {
  title: string
  engine: string
  proxy: string
  why: string
}

export const DIVERGENCES: Divergence[] = [
  {
    title: 'self-invoke under a prefix',
    engine: 'a prefixed worker invoking its own bare foo gets NOT_FOUND.',
    proxy: 'resolves the bare id to the session’s own {prefix}::foo so the call succeeds.',
    why: 'a worker should be able to call what it registered. opt-out for strict parity.',
  },
  {
    title: 'denied-discovery error code',
    engine: 'distinguishes FORBIDDEN (denied) from NOT_FOUND (missing), leaking existence.',
    proxy: 'defaults to FORBIDDEN parity; may opt into collapsing denied → NOT_FOUND.',
    why: 'hardening for hostile multi-tenant discovery.',
  },
  {
    title: 'observer targets and sources',
    engine: 'checks a session’s access to an observer’s function and target.',
    proxy: 'runs the same checks, and refuses engine:: targets and sources whose calls list functions, workers or observers.',
    why: 'a proxied connection has no engine session: an engine:: target would run unrestricted.',
  },
  {
    title: 'call-chain claims',
    engine: 'keeps the caller an sdk sets on a call or a report.',
    proxy: 'keeps it only when it names a call the engine routed to that connection, and writes it from that call.',
    why: 'observers filter on caller; a worker must not place its calls in another worker’s chain.',
  },
]
